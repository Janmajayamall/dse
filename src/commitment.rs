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

#[derive(Debug)]
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

struct Handler {
    request: Request,
    counter_wallet_address: Address,
    commitments_sent: HashMap<u32, String>,
    commitments_received: HashMap<u32, String>,
    stage: Stage,
    network_client: network::Client,
    command_receiver: mpsc::Receiver<Command>,
    parent_client: Client,
}

impl Handler {
    pub fn new(
        request: Request,
        counter_wallet_address: Address,
        network_client: network::Client,
        command_receiver: mpsc::Receiver<Command>,
        parent_client: Client,
    ) -> Self {
        Self {
            request,
            counter_wallet_address,
            commitments_sent: Default::default(),
            commitments_received: Default::default(),
            stage: Stage::Unitialised,
            network_client,
            command_receiver,
            parent_client
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
                                        // Only respond to commit request when 
                                        // round is <= to curr_round
                                        if query_id == self.request.query_id() && round <= self.curr_round() {
                                            match self.get_commitment_for_round(round).await {
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
                                    // Recevied invalidting signature from requester
                                    network::CommitRequest::InvalidatingSignature {
                                        query_id,
                                        invalidating_signature
                                    } => {
                                        // TODO check invalidating signature is correct
                                        // Evict all the commitments

                                        self.network_client.send_dse_message_response(request_id, network::DseMessageResponse::Commit(
                                            network::CommitResponse::AckInvalidatingSignature(query_id)
                                        )).await;
                                    }
                                    _ => {}
                                }
                            },
                            // Command to be used by service requester to produce
                            // invalidating signatures
                            Command::ProduceInvalidatingSignature(query_id) => {
                                // Only service requester can produce invalidating signature
                                if query_id == self.request.query_id() && self.request.is_requester == true {
                                    // TODO produce invalidating signature & send it country party   

                                    match self.network_client.send_dse_message_request(self.request.counter_party_peer_id(), network::DseMessageRequest::Commit(
                                        network::CommitRequest::InvalidatingSignature {
                                            query_id,
                                            invalidating_signature: "dwiaodja".into(),
                                        }
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
                // figure out the index to ask for 
                let round = self.curr_round();
                
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
                                        // TODO check that commitment is valid
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
                // ask or counter wallet address
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

    /// lastest round for which commitment 
    /// is pending from the counter party
    fn curr_round(&self) -> u32 {
        for n in 0..self.rounds() {
            // provider only sends commitments in odd rounds
            // since it only has to commit half as much as requester
            if (self.request.is_requester == true && n % 2 == 1) || self.request.is_requester == false {
                if self.commitments_received.contains_key(&n) == false {
                    return n;
                }
            }
        }
        self.rounds()
    }

    fn rounds(&self) -> u32 {
        // TODO: rn we have hardcoded that the per index value
        // is 1*10**16 (i.e. cent). Make it configurable.
        (self.request.bid.bid.charge / U256::from_str("10000000").unwrap()).as_u32()
    }

    async fn get_commitment_for_round(&mut self, round: u32) -> Result<String, anyhow::Error> {
        match self.commitments_sent.get(&round) {
            Some(val) => Ok(val.clone()),
            None => {
                let commitment = self.parent_client.handle_find_commitment(self.request.query_id()).await?;
                Ok(commitment)
            }
        }
    }
}

#[derive(Debug)]
enum Command {
    AddRequest(Request),
    ReceivedRequest{
        request: network::CommitRequest,
        request_id: request_response::RequestId,
    },
    FindCommitment { 
        query_id: indexer::QueryId,
        sender: oneshot::Sender<Result<String, anyhow::Error>>,
    },
    ProduceInvalidatingSignature(indexer::QueryId),
}

struct Client {
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

    pub async fn handle_find_commitment(&mut self, query_id: indexer::QueryId) -> Result<String, anyhow::Error> {
        let (sender, receiver) = oneshot::channel();    
        self.command_sender.send(
            Command::FindCommitment { query_id, sender }
        ).await.expect("commitments: Command sender message dropped");
        receiver.await.expect("commitments: Response message dropped")
    }

    pub async fn handle_produce_invalidating_signature(&mut self, query_id: indexer::QueryId) -> Result<String, anyhow::Error> {
        let (sender, receiver) = oneshot::channel();    
        self.command_sender.send(
            Command::ProduceInvalidatingSignature(query_id)
        ).await.expect("commitments: Command sender message dropped");
        receiver.await.expect("commitments: Response message dropped")
    }
}

struct Commitment {
    pending_requests: HashMap<indexer::QueryId, mpsc::Sender<Command>>,
    command_receiver: mpsc::Receiver<Command>,
    // event_sender: mpsc::Sender<CommitmentEvent>,
    network_client: network::Client,
    unused_indexes: VecDeque<u32>,
    used_indexes: HashMap<indexer::QueryId, VecDeque<u32>>,
    index_invalidating_rec: HashMap<u32, String>,
    index_value: U256,
}

impl Commitment {
    pub fn new(
        command_receiver: mpsc::Receiver<Command>,
        network_client: network::Client,
        index_value: U256
    ) -> Self {
        Self {
            pending_requests: Default::default(),
            command_receiver,
            network_client,
            unused_indexes: Default::default(),
            used_indexes: Default::default(),
            index_invalidating_rec: Default::default(),
            index_value
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
                        Some(c) => {
                            match c {
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
                                    // let (sender, receiver) = mpsc::
                                    // request.query_id()
                                    // 2. open a channel

                                },
                                Command::FindCommitment {
                                    query_id,
                                    sender
                                } => {
                                    match self.find_index(query_id) {
                                        Some(v) => {
                                            // TODO sign the message and send the commitment
                                            sender.send(Ok("FakeRes".into()))
                                        },
                                        None => {
                                            sender.send(Err(anyhow::anyhow!("Commitments: Err - no index left")))
                                        }
                                    }
                                    ;
                                },
                                Command::ProduceInvalidatingSignature(query_id) => {
                                    // TODO notify handlers
                                }
                                _ => {}
                            }
                        },
                        None => {}
                    }
                },
            }
        }
    }

    pub fn find_index(&mut self, query_id: indexer::QueryId) -> Option<u32> {
        let value = self.unused_indexes.pop_front();
        if let Some(v) = value {
            self.mark_index_in_use(query_id, v);
        }
        value
    }

    pub fn mark_index_in_use(&mut self, query_id: indexer::QueryId, index: u32) {
        match self.used_indexes.get(&query_id) {
            Some(vec) => {
                vec.push_back(index);
            },
            None => {
                let v: VecDeque<u32> = VecDeque::new();
                v.push_back(index);
                self.used_indexes.insert(query_id, v);
            }
        }
    }

    pub fn mark_query_indexes_unused(&mut self, query_id: indexer::QueryId) {
        if let Some(vals) = self.used_indexes.remove(&query_id) {
            for i in vals.iter() {
                self.unused_indexes.push_back(i.clone());
            }
        };
    }

    // range usage is disabled for now
    pub fn find_index_range(&mut self, amount: U256) -> Option<(u32, u32)> {
        let start: u32;
        let end: u32;
        for i in self.unused_indexes.iter() {
            if (U256::from(end - start) * self.index_value) == amount {

                return Some((start, end));
            }
            if end == i - 1 {
                end += 1;
            } else {
                start = *i;
                end = *i;
            }
        };
        None
    }
    

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
