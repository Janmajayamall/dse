use ethers::{types::*};
use libp2p::{PeerId, Multiaddr};
use std::collections::{HashMap, VecDeque};
use std::str::FromStr;
use tokio::sync::{mpsc};
use tokio::{select, time};

use super::indexer;
use super::network;


enum Stage {
    Loaded,
    Rounds,
    Success,
}

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
    event_sender: mpsc::Sender<CommitmentEvent>,
    stage: Stage,
    network_client: network::Client,
}

impl Handler {
    pub async fn start(
        mut self,
    ) {
    
        loop {
            let mut interval = time::interval(time::Duration::from_secs(10));
            interval.tick().await;
            if self.counter_wallet_address != Address::default() {
                // figure out the index to ask for 
                let round = self.curr_round();
                
                if round == self.rounds() {
                    // send round request
                    match self.network_client.send_dse_message_request(self.request.counter_party_peer_id(), network::DseMessageRequest::EndCommit(self.request.query_id())).await {
                        Ok(res) => {
                            network::DseMessageResponse::AckEndCommit(query_id) {
                                if query_id == self.request.query_id() {
                                  // TODO send end commit event to indexer to proceed further
                                  // Also, advance the state
                                }
                            },
                            _ => {}
                        },
                       Err(_) => {}
                    }
                }else {
                    // send round request
                    match self.network_client.send_dse_message_request(self.request.counter_party_peer_id(), network::DseMessageRequest::CommitFund { query_id: self.request.query_id(), round}).await {
                        Ok(res) => {
                            network::DseMessageResponse::CommitFund { query_id, round, commitment } => {
                                // TODO check that commitment is valid
                                if query_id == self.request.query_id() {
                                  self.commitments_received.insert(round, commitment);
                                }
                            },
                            _ => {}
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
                
                // check what next message to send
                // find the indexes available (how to find the index?)

                // get the index
                // market it as used
                // sign the message, along with epoch


                // and then send it

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
}

struct Commitment {
    pending_requests: HashMap<indexer::QueryId, Request>,
    command_receiver: mpsc::Receiver<Command>,
    event_sender: mpsc::Sender<CommitmentEvent>,
    unused_indexes: VecDeque<u32>,
    used_indexes: HashMap<indexer::QueryId, Box<[u32]>>,
    index_invalidating_rec: HashMap<u32, String>,
    index_value: U256,
}

impl Commitment {
    pub fn new(
        command_receiver: mpsc::Receiver<Command>,
        index_value: U256
    ) -> Self {
        Self {
            pending_requests: Default::default(),
            command_receiver,
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

    pub async fn run(mut self) {
        loop {
           
            select! {
                command = self.command_receiver.recv() => {
                    match command {
                        _ => {}
                    }
                },
            }
            
           
        }
    }

    pub fn find_index(mut self) -> Option<u32> {
        self.unused_indexes.pop_front()
    }

    pub fn mark_index_in_use(mut self, query_id: indexer::QueryId, indexes: 
        Box<[u32]>) {
        self.used_indexes.insert(query_id, indexes);
    }

    pub fn mark_query_indexes_unused(mut self, query_id: indexer::QueryId) {
        if let Some(vals) = self.used_indexes.remove(&query_id) {
            for i in vals.iter() {
                self.unused_indexes.push_back(i.clone());
            }
        };
    }

    // range usage is disabled for now
    pub fn find_index_range(mut self, amount: U256) -> Option<(u32, u32)> {
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