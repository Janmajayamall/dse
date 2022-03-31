use ethers::{types::*};
use libp2p::{request_response};
use libp2p::{PeerId, Multiaddr};
use std::collections::{HashMap, VecDeque};
use std::str::FromStr;
use tokio::sync::{mpsc, oneshot};
use tokio::{select, time};

use super::indexer;
use super::network;

enum Stage {
    // Waiting for countr party wallet address
    Unitialised,
    // Commit rounds going on
    Commit,
    // Waiting for service to go through
    WaitingForService,
    // Waiting for invalidation signature
    WaitingInvalidationSignature,
    // Commit Ended
    CommitEnded
}

#[derive(Debug, Clone)]
struct Request {  
    is_requester: bool,
    bid: indexer::BidReceived,
    query: indexer::QueryReceived,
}

impl Request {
    fn counter_party_peer_id(&self) -> PeerId {
        if self.is_requester == true {
            return self.bid.bidder_id;
        };
        return self.query.requester_id;
    }   

    fn counter_party_multiaddress(&self) -> Multiaddr {
        if self.is_requester == true {
            return self.bid.bidder_addr;
        };
        return self.query.requester_addr;
    }

    fn query_id(&self) -> indexer::QueryId {
        self.query.id
    }
}

pub enum CommitmentEvent {
    SendDseMessageRequest {
        request: network::DseMessageRequest,
        send_to: PeerId,
    },
}

#[derive(Debug)]
pub enum HandlerEvent {
    // Find a valid index for commitment
    FindCommitmentIndex { 
        query_id: indexer::QueryId,
        sender: oneshot::Sender<Result<u32, anyhow::Error>>,
    },
    // Add invalidating signature of the query id
    AddInvalidatingSignature {
        query_id: indexer::QueryId,
        signature: String,
    },

}

struct Handler {
    request: Request,
    counter_wallet_address: Address,
    commitments_sent: HashMap<u32, String>,
    commitments_received: HashMap<u32, String>,
    stage: Stage,
    network_client: network::Client,
    command_receiver: mpsc::Receiver<Command>,
    handler_sender: mpsc::Sender<HandlerEvent>,
}

#[derive(Debug)]
enum RoundType {
    // Commitment Type 1
    T1,
    // Commitment Type 2
    T2
}

impl Handler {
    pub fn new(
        request: Request,
        network_client: network::Client,
        command_receiver: mpsc::Receiver<Command>,
        handler_sender: mpsc::Sender<HandlerEvent>,
    ) -> Self {
        Self {
            request,
            counter_wallet_address: Address::zero(),
            commitments_sent: Default::default(),
            commitments_received: Default::default(),
            stage: Stage::Unitialised,
            network_client,
            command_receiver,
            handler_sender,
        }
    }

    pub async fn start(
        mut self,
    ) {
        loop {
            select! {
                command = self.command_receiver.recv() => {
                    if let Some(c) = command {
                        match c {
                            Command::ReceivedRequest {
                                request_id,
                                request,
                            } => {
                                match request {
                                    network::CommitRequest::CommitFund {
                                        query_id,
                                        round,
                                    } => {
                                        if query_id == self.request.query_id() && self.is_valid_round_request(round) {
                                            match self.find_commitment_for_round(round, self.round_commitment_type(round, self.request.is_requester)).await {
                                                Ok(commitment) => {
                                                    // send response to the request
                                                    self.network_client.send_dse_message_response(request_id, network::DseMessageResponse::Commit(
                                                        network::CommitResponse::CommitFund {
                                                            query_id,
                                                            round,
                                                            commitment
                                                        }
                                                    )).await;
                                                },
                                                Err(_) => {
                                                    // TOOD: unable to find index to commit.
                                                    // Behaviour for this case is still unknown
                                                }
                                            }
                                        }
                                    },
                                    // Recevied invalidting signature from service requester
                                    network::CommitRequest::InvalidatingSignature {
                                        query_id,
                                        invalidating_signature
                                    } => {
                                        // TODO check invalidating signature is correct for every commitment
                                        // Evict all the commitments

                                        self.handler_sender.send(HandlerEvent::AddInvalidatingSignature {
                                            query_id,
                                            signature: invalidating_signature,
                                        }).await;

                                        self.network_client.send_dse_message_response(request_id, network::DseMessageResponse::Commit(
                                            network::CommitResponse::AckInvalidatingSignature(query_id)
                                        )).await;

                                        // TODO END the handler
                                    }
                                    _ => {}
                                }
                            },
                            // Command to be used by service requester to produce
                            // invalidating signatures for the query
                            Command::ProduceInvalidatingSignature(query_id) => {
                                // Only service requester can produce invalidating signature
                                if query_id == self.request.query_id() && self.request.is_requester == true {
                                    // TODO produce invalidating signature & send it country party   
                                    let invalidating_signature: String = "FakeSiganture".into();

                                    self.handler_sender.send(HandlerEvent::AddInvalidatingSignature {
                                        query_id,
                                        signature: invalidating_signature.clone(),
                                    }).await;

                                    match self.network_client.send_dse_message_request(self.request.counter_party_peer_id(), network::DseMessageRequest::Commit(
                                        network::CommitRequest::InvalidatingSignature {
                                            query_id,
                                            invalidating_signature,
                                        }

                                        // TODO END the handler
                                    )).await {
                                        Ok(res) => {},
                                        Err(_) => {
                                            // TODO probably try again? 
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            } 
            
            let mut interval = time::interval(time::Duration::from_secs(10));
            interval.tick().await;

            if self.counter_wallet_address != Address::default() {
                // figure out the index to ask for (i.e. round)
                let round = self.next_expected_round();
                
                if round == self.rounds() {
                    if self.request.is_requester == true {
                        // TODO: notify main to wait for service
                    }else {
                        // TODO: notify main to provide service
                    }
                }else {
                    // send round request
                    match self.network_client.send_dse_message_request(self.request.counter_party_peer_id(), network::DseMessageRequest::Commit(network::CommitRequest::CommitFund { query_id: self.request.query_id(), round} )).await {
                        Ok(res) => {
                            match res {
                                network::DseMessageResponse::Commit(network::CommitResponse::CommitFund {
                                    query_id,
                                    round,
                                    commitment
                                }) => {
                                        // TODO 
                                        // check that the commitment is valid
                                        // and is of valid type
                                        
                                        if query_id == self.request.query_id() {
                                            self.commitments_received.insert(round, commitment);
                                        }
                                },
                                _ => {}
                            }
                        },
                       Err(_) => {}
                    }
                }
            }else {
                // ask for counter wallet address
                match self.network_client.send_dse_message_request(self.request.counter_party_peer_id(),  network::DseMessageRequest::WalletAddress(self.request.query_id())).await {
                    Ok(res) => {
                        match res {
                            network::DseMessageResponse::WalletAddress { query_id, wallet_address } => {
                                if query_id == self.request.query_id() {
                                    self.counter_wallet_address = wallet_address;
                                }
                            },
                            _ => {}
                        };
                    },
                    Err(_)=> {}
                }
            }
        }
    }

    /// Returns true if counter party has asked for 
    /// commitment of a valid round.
    /// A provider only commits in odd rounds, whereas
    /// request commits in all rounds.
    /// Commit request for a round is invalid if 
    /// round > expected round
    fn is_valid_round_request(&self, r_round: u32) -> bool {
        // provider does not responds to even rounds
        if self.request.is_requester == false && r_round % 2 == 0 {
            return false;
        }

        if r_round > self.next_expected_round() {
            return false;
        }

        return true;
    }

    /// Lastest round for which commitment 
    /// is pending from the counter party
    fn next_expected_round(&self) -> u32 {
        for n in 0..self.rounds() {
            // provider only sends commitments in odd rounds
            // since it only has to commit half as much as requester
            if (self.request.is_requester == true && n % 2 == 1) || self.request.is_requester == false {
                if self.commitments_received.contains_key(&n) == false {
                    return n;
                }
            }
        }
        // recieved all commitments
        self.rounds()
    }

    /// Returns round commitment type (either T1 or T2).
    /// All commitments by service provider are of T1, whereas
    /// first 50% (i.e. commitments in round < total_rounds / 2)
    /// commitments from requester are of T1 and last 50%
    /// are of T2.
    fn round_commitment_type(&self, round: u32, is_requester: bool) -> RoundType {
        // requester's commitments are of type 2 after round/2
        if is_requester == true && round  >= (self.rounds()/2) {
            return RoundType::T2
        }

        return RoundType::T1;
    }

    /// Returns total number of commitment rounds
    /// required for this exchange. 
    /// total_rounds = bid.charge * 2. This is
    /// because amount that is to be committed by 
    /// requester is bid.charge * 2. 
    /// provider only commits amount = bid.charge.
    fn rounds(&self) -> u32 {
        // TODO: rn we have hardcoded that the per index value
        // is 1*10**16 (i.e. cent). Make it configurable.
        // Amount to be committed by requester = charge * 2
        // Amount to be committed by provider = charge 
        ((self.request.bid.bid.charge * U256::from_str("2").unwrap()) / U256::from_str("10000000").unwrap()).as_u32()
    }

    // Returns commitment of the node 
    // for a given round.
    async fn find_commitment_for_round(&mut self, round: u32, c_type: RoundType) -> Result<String, anyhow::Error> {
        // look whether there already exists 
        // commitment for the round
        match self.commitments_sent.get(&round) {
            Some(val) => Ok(val.clone()),
            None => {
                let (sender, receiver) = oneshot::channel::<Result<u32, anyhow::Error>>();
                self.handler_sender.send(
                    HandlerEvent::FindCommitmentIndex { 
                        query_id: self.request.query_id(), 
                        sender
                    }
                ).await?;
                let index = receiver.await??;

                // TODO create commitment by c_type for the index 
                let commitment: String = "FakeOne" .into();
                self.commitments_sent.insert(round, commitment.clone());
                Ok(commitment)
            }
        }
    }

}

#[derive(Debug)]
enum Command {
    // Process new request in new handler
    AddRequest(Request),
    // Received a request over network
    ReceivedRequest{
        request: network::CommitRequest,
        request_id: request_response::RequestId,
    },
    // Produces invalidting signature (used by service requester)
    ProduceInvalidatingSignature(indexer::QueryId),
}

#[derive(Clone)]
pub struct Client {
    command_sender: mpsc::Sender<Command>,
}   

impl Client {
    pub fn new(
        command_sender: mpsc::Sender<Command>,
    ) -> Self {
        Self {
            command_sender,
        }
    }

    pub fn handle_add_request() {

    }

    // pub async fn handle_find_commitment(&mut self, query_id: indexer::QueryId, c_type: RoundType) -> Result<String, anyhow::Error> {
    //     let (sender, receiver) = oneshot::channel();    
    //     self.command_sender.send(
    //         Command::FindCommitment { query_id, c_type, sender }
    //     ).await.expect("commitments: Command sender message dropped");
    //     receiver.await.expect("commitments: Response message dropped")
    // }

    pub async fn handle_produce_invalidating_signature(&mut self, query_id: indexer::QueryId) -> Result<String, anyhow::Error> {
        let (sender, receiver) = oneshot::channel();    
        self.command_sender.send(
            Command::ProduceInvalidatingSignature(query_id)
        ).await.expect("commitments: Command sender message dropped");
        receiver.await.expect("commitments: Response message dropped")
    }
}

struct Commitment {
    // Receives commands
    command_receiver: mpsc::Receiver<Command>,
    // Network client
    network_client: network::Client,
    // Used for handlers to send events
    handler_sender: mpsc::Sender<HandlerEvent>,
    // Receives events from all handlers
    handler_receiver: mpsc::Receiver<HandlerEvent>,
    // Requests under process
    pending_requests: HashMap<indexer::QueryId, mpsc::Sender<Command>>,
    // Indexes not used for commitment
    unused_indexes: VecDeque<u32>,
    // Indexes used for T1 commitments by query
    type1_used_indexes: HashMap<indexer::QueryId, VecDeque<u32>>,
    // Indexes used for T2 commitments by query
    type2_used_indexes: HashMap<indexer::QueryId, VecDeque<u32>>,
    // T1 commitmnts received 
    type1_recv_commitments: HashMap<indexer::QueryId, VecDeque<String>>,
    // T2 commitments received
    type2_recv_commitments: HashMap<indexer::QueryId, VecDeque<String>>,
    // Invalidating signatures by query id
    invalidating_signatures: HashMap<indexer::QueryId, String>,
    // TODO create a struct of commitment that stores the message as well as the signture
    // TODO we haven't taken care of storing invalidating signature history per index
}

impl Commitment {
    pub fn new(
        command_receiver: mpsc::Receiver<Command>,
        network_client: network::Client,
        client: Client,
        index_value: U256
    ) -> Self {
        let (handler_sender, handler_receiver) = mpsc::channel(10);

        Self {
            command_receiver,
            network_client,
            handler_sender,
            handler_receiver,
            pending_requests: Default::default(),
            unused_indexes: Default::default(),
            type1_used_indexes: Default::default(),
            type2_used_indexes: Default::default(),
            type1_recv_commitments: Default::default(),
            type2_recv_commitments: Default::default(),
            invalidating_signatures: Default::default(),
        }
    }

    pub fn new_commitment(bid: indexer::BidReceived, query: indexer::QueryReceived,is_requester: bool) {
        let request = Request {
            is_requester,
            bid,
            query,
        };
    }

    pub async fn run(&mut self) {
        loop {
            select! {
                command = self.command_receiver.recv() => {
                    match command {
                        Some(c) => {self.command_handler(c).await},
                        None => {}
                    }
                },
                event = self.handler_receiver.recv() => {
                    match event {
                        Some(e) => {self.handler_events(e).await},
                        None => {}
                    }
                }
            }
        }
    }

    pub async fn command_handler(&mut self, command: Command) {
        match command {
            Command::ReceivedRequest {
                request_id,
                request,
            } => {
                match request {
                    network::CommitRequest::CommitFund {
                        query_id,
                        round
                    } => {
                        
                    },
                    network::CommitRequest::EndCommit(query_id) => {
                        // check whether commit has ended; if yes, then send yes

                    },
                    _ => {}
                }
            },
            Command::AddRequest(request)  => {
                // create new handler
                let (sender, receiver) = mpsc::channel::<Command>(10);
                // insert sender for handler to pending requests
                self.pending_requests.insert(request.query_id().clone(), sender);
                let handler = Handler::new(request, self.network_client.clone(), receiver, self.handler_sender.clone());
                tokio::spawn(handler.start());
            },
            Command::ProduceInvalidatingSignature(query_id) => {
                // TODO notify handlers
            },
            _ => {}
        }
    }

    pub async fn handler_events(&mut self, event: HandlerEvent) {
        match event {
            HandlerEvent::FindCommitmentIndex {
                    query_id,
                    sender
                } => {
                    match self.find_index(query_id) {
                        Some(v) => {
                            sender.send(Ok(v))
                        },
                        None => {
                            sender.send(Err(anyhow::anyhow!("Commitments: Err - no index left")))
                        }
                    }
                    ;
            },
            HandlerEvent::AddInvalidatingSignature {
                    query_id, 
                    signature,
                }  => {

                // Free up indexes used for
                // type 1 commitment by query id.
                // We don't touch indexes used for
                // type 2 commitments, since they are
                // spent irrespective of invaldiating 
                // signature.
                match self.type1_used_indexes.remove(&query_id) {
                    Some(vec)=> {
                        for i in vec.iter() {
                            self.unused_indexes.push_back(i.clone());
                        }
                    },
                    None => {}
                }

                // Delete type 1 commitments received
                // since they aren't valid anymore.
                // We don't touch type 2 commitments received 
                // for this query, since they have been 
                // finalised (i.e. can be redeemed on-chain)
                self.type1_recv_commitments.remove(&query_id);

                // store the invalidating signature for the query id
                self.invalidating_signatures.insert(query_id, signature);
            },
        }
    }

    pub fn find_index(&mut self, query_id: indexer::QueryId) -> Option<u32> {
        self.unused_indexes.pop_front()
    }

    /// Stores indexes used for type 1 
    /// and type 2 commitments by query id 
    pub fn mark_index_in_use(&mut self, query_id: indexer::QueryId, index: u32, c_type: RoundType) {
        match c_type {
            RoundType::T1 => {
                match self.type1_used_indexes.get(&query_id) {
                    Some(vec) => {
                        vec.push_back(index);
                    },
                    None => {
                        let v: VecDeque<u32> = VecDeque::new();
                        v.push_back(index);
                        self.type1_used_indexes.insert(query_id, v);
                    }
                }
            },
            RoundType::T2 => {
                match self.type2_used_indexes.get(&query_id) {
                    Some(vec) => {
                        vec.push_back(index);
                    },
                    None => {
                        let v: VecDeque<u32> = VecDeque::new();
                        v.push_back(index);
                        self.type1_used_indexes.insert(query_id, v);
                    }
                }
            }
        }
    }

    // Wallet

    // // range usage is disabled for now
    // pub fn find_index_range(&mut self, amount: U256) -> Option<(u32, u32)> {
    //     let start: u32;
    //     let end: u32;
    //     for i in self.unused_indexes.iter() {
    //         if (U256::from(end - start) * self.index_value) == amount {

    //             return Some((start, end));
    //         }
    //         if end == i - 1 {
    //             end += 1;
    //         } else {
    //             start = *i;
    //             end = *i;
    //         }
    //     };
    //     None
    // }
}


// Progrssive commitment 
// 1
// 1
//     1
// 1
// 1
//     1
// 1
// 1
//     1
// 1
// 1
//     1


// issues
// How to handle commitment of type 1 and 2
// How to make sure handle commitments are trsmitted to global storag

// Figure out whether handler or commitment will take care of commitment signature
// Handler needs to validate & sign