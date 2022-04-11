use ethers::{
    types::{Address, Signature, H256, U256},
    utils::keccak256,
};
use libp2p::{Multiaddr, PeerId};
use serde::{Deserialize, Serialize};
use sled::{Config, Db};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type QueryId = u32;

use super::network_client;

/// Stores qeury string and
/// other realted info to the query
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct QueryData {
    query_string: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct Query {
    pub id: QueryId,
    pub data: QueryData,
    pub requester_addr: Multiaddr,
    pub requester_id: PeerId,
}

impl Query {
    pub async fn from_data(
        data: QueryData,
        mut network_client: network_client::Client,
    ) -> anyhow::Result<Self> {
        network_client
            .network_details()
            .await
            .and_then(|(requester_id, requester_addr)| {
                Ok(Query {
                    id: Default::default(),
                    data,
                    requester_id,
                    requester_addr,
                })
            })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BidData {
    query_id: QueryId,
    charge: ethers::types::U256,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct Bid {
    pub query_id: QueryId,
    pub provider_addr: Multiaddr,
    pub provider_id: PeerId,
    pub charge: U256,
}

impl Bid {
    pub async fn from_data(
        data: BidData,
        mut network_client: network_client::Client,
    ) -> anyhow::Result<Self> {
        network_client
            .network_details()
            .await
            .and_then(|(provider_id, provider_addr)| {
                Ok(Bid {
                    query_id: data.query_id,
                    provider_id,
                    provider_addr,
                    charge: data.charge,
                })
            })
    }
}

#[derive(Deserialize, Serialize, Debug, PartialEq)]
// P = BidAccepted
// R = Waiting Start Commit
// P = Waiting T1 Commit
// R = Waitinf P's T1 Commit
// P = Waiting T2 commit
// R = Waiting Service
// P = Waiting Invalidating Singauture
pub enum TradeStatus {
    /// Provider should send StartCommit
    PSendStartCommit,
    /// Requester is waiting for Start Commit
    /// from provider
    WaitingStartCommit,
    /// Provider is waiting for Requester's T1
    /// Commit
    WaitingRT1Commit,
    /// Requester should send T1 commit
    RSendT1Commit,
    /// Requester is waiting for Provider's T1
    /// Commit
    WaitingPT1Commit,
    /// Provider should send T1 commit
    PSendT1Commit,
    /// Provider is waiting for Requester's T2
    /// Commit
    WaitingRT2Commit,
    /// Requester should send T2 commit
    RSendT2Commit,
    /// Requester is waiting for Provider to
    /// fulfill the service
    WaitingForService,
    /// Provider should provide service
    PFulfillService,
    /// Provider is waitig for Invalidting Signatures
    WaitingInvalidatingSignatures,
}

/// Stores trade infor between
/// node and some peer
#[derive(Deserialize, Serialize, Debug)]
pub struct Trade {
    /// Trade query
    pub query: Query,
    /// Trade bid (accepted by requester)
    pub bid: Bid,
    /// Status of the trade
    pub status: TradeStatus,
    /// Query id
    pub query_id: QueryId,
    /// Flag whether node is requester
    /// OR provider
    pub is_requester: bool,
    /// Wallet address of requester
    pub requester_wallet_address: Address,
    /// Wallet address of provider
    pub provider_wallet_address: Address,
}

impl Trade {
    pub fn t1CommitAmount(&self) -> U256 {
        self.bid.charge.div_mod(U256::from_dec_str("2").unwrap()).0
    }

    pub fn t2CommitAmount(&self) -> U256 {
        self.bid.charge
    }

    pub fn update_waiting_status(&mut self) {
        match self.status {
            TradeStatus::WaitingStartCommit => {
                self.status = TradeStatus::RSendT1Commit;
            }
            TradeStatus::WaitingRT1Commit => {
                self.status = TradeStatus::PSendT1Commit;
            }
            TradeStatus::WaitingPT1Commit => {
                self.status = TradeStatus::RSendT2Commit;
            }
            TradeStatus::WaitingRT2Commit => {
                self.status = TradeStatus::PFulfillService;
            }
            _ => {}
        }
    }

    pub fn update_sending_status(&mut self) {
        match self.status {
            TradeStatus::PSendStartCommit => {
                self.status = TradeStatus::WaitingRT1Commit;
            }
            TradeStatus::RSendT1Commit => {
                self.status = TradeStatus::WaitingPT1Commit;
            }

            TradeStatus::PSendT1Commit => {
                self.status = TradeStatus::WaitingRT2Commit;
            }
            TradeStatus::RSendT2Commit => {
                self.status = TradeStatus::WaitingForService;
            }
            _ => {}
        }
    }

    pub fn is_sending_status(&self) -> bool {
        self.status == TradeStatus::PSendStartCommit
            || self.status == TradeStatus::RSendT1Commit
            || self.status == TradeStatus::PSendT1Commit
            || self.status == TradeStatus::RSendT2Commit
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
enum CommitType {
    /// Commitment Type 1
    T1 = 1,
    /// Commitment Type 2
    T2 = 2,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Commit {
    /// index used for the commitment
    index: u32,
    /// epoch during which the commitment is valid
    epoch: u32,
    /// unique id that identifies commitment with the
    /// corresponding query id
    u: u32,
    /// commit type 1 or 2
    c_type: CommitType,
    /// address that invalidates the commitment
    i_address: Address,
    /// address that can redeeem this commitment
    /// with invalidating signature.
    /// Only needed for c_type = 2
    r_address: Address,
    /// owner's signature on commitment's blob
    signature: Option<Signature>,
}

impl Commit {
    pub fn commit_hash(&self) -> H256 {
        let mut blob: [u8; 160] = [0; 160];
        U256::from(self.index).to_little_endian(&mut blob[32..64]);
        U256::from(self.epoch).to_little_endian(&mut blob[64..96]);
        U256::from(self.u).to_little_endian(&mut blob[96..128]);
        U256::from(self.c_type.clone() as u32).to_little_endian(&mut blob[128..160]);

        let mut blob: Vec<u8> = Vec::from(blob);

        blob.extend_from_slice(self.i_address.as_bytes());

        // r_address is added for commitment type 2
        if self.c_type == CommitType::T2 {
            blob.extend_from_slice(self.r_address.as_bytes());
        }

        keccak256(blob.as_slice()).into()
    }
}

/// tree that stores trades for every query

const OLD_TRADES: &[u8] = b"old-trades";
const ACTIVE_TRADES: &[u8] = b"active-trades";

const QUERIES_SENT: &[u8] = b"queries-sent";
const QUERIES_RECEIVED: &[u8] = b"queries-received";

const BIDS_SENT: &[u8] = b"bids-sent";
const BIDS_RECEIVED: &[u8] = b"bids-received";

const T1_COMMITS_RECEIVED: &[u8] = b"t1-commits-received";
const T2_COMMITS_RECEIVED: &[u8] = b"t2-commits-received";

pub struct Storage {
    /// Keeps track of unused indexes
    unused_indexes: u32,
    /// Stores all graphs
    ///
    /// Graphs include - AllTrades, ActiveTrades
    /// QueriesSent
    db: Mutex<Db>,
}

impl Storage {
    // functions
    // maintain DB for queries sent
    // fn for adding new Trade
    // fn for updating Trade status

    /// Adds newly received bid to query's
    /// bid tree
    pub fn add_bid_received_for_query(&self, query_id: &QueryId, bid: Bid) {
        let mut db = self.db.lock().unwrap();

        let bids = db
            .open_tree([BIDS_RECEIVED, &query_id.to_be_bytes()].concat())
            .expect("db: failed to open tree");

        // BidderId uniquely identifies a bid received for
        // the query sent
        bids.insert(
            bid.provider_id.to_bytes(),
            bincode::serialize(&bid).unwrap(),
        );
    }

    /// Adds bid placed by the provider (i.e. node) for a
    /// query id
    pub fn add_bid_sent_for_query(&self, bid: Bid, query: Query) {
        let mut db = self.db.lock().unwrap();

        let trades = db.open_tree(BIDS_SENT).expect("db: failed to open tree");

        // QueryId uniquely identifies bid sent for
        // a query received
        trades.insert(query.id.to_be_bytes(), bincode::serialize(&bid).unwrap());
    }

    /// Adds new query published by the requester (i.e. node)
    pub fn add_query_sent(&self, query: Query) {
        let mut db = self.db.lock().unwrap();
        let queries = db.open_tree(QUERIES_SENT).expect("db: failed to open tree");

        // TODO: check that you are not overwriting
        // previous query with same id.

        queries.insert(query.id.to_be_bytes(), bincode::serialize(&query).unwrap());
    }

    /// Adds new query received by the provider over gossipsub
    pub fn add_query_received(&self, query: Query) {
        let mut db = self.db.lock().unwrap();
        let queries = db
            .open_tree(QUERIES_RECEIVED)
            .expect("db: failed to open tree");

        // TODO: check that you are not overwriting
        // previous query with same id.

        queries.insert(query.id.to_be_bytes(), bincode::serialize(&query).unwrap());
    }

    /// Adds new trade for a query and a bid
    ///
    /// Call after requester accepts the bid
    pub fn add_new_trade(&self, query: Query, bid: Bid, is_requester: bool) {
        let mut db = self.db.lock().unwrap();
        let trades = db
            .open_tree(ACTIVE_TRADES)
            .expect("db: failed to open tree");
        let trade = Trade {
            query,
            bid,
            status: if is_requester {
                TradeStatus::WaitingStartCommit
            } else {
                TradeStatus::PSendStartCommit
            },
            query_id: query.id,
            is_requester,
        };
        trades.insert(query.id.to_be_bytes(), bincode::serialize(&trade).unwrap());
    }

    /// Updates Active Trade
    ///
    /// Returns err if Trade does not pre-exists.
    pub fn update_active_trade(&self, trade: Trade) -> anyhow::Result<()> {
        self.find_active_trade(
            &trade.query_id,
            &trade.bid.provider_id,
            &trade.query.requester_id,
        )
        .and_then(|_| {
            let mut db = self.db.lock().unwrap();
            db.open_tree(ACTIVE_TRADES)
                .expect("db: failed to open tree")
                .insert(
                    trade.query_id.to_be_bytes(),
                    bincode::serialize(&trade).unwrap(),
                );
            Ok(())
        })
    }

    /// Get all active trades
    pub fn get_active_trades(&self) -> Vec<Trade> {
        let mut db = self.db.lock().unwrap();
        let trades = db
            .open_tree(ACTIVE_TRADES)
            .expect("db: failed to open tree");
        trades
            .iter()
            .filter_map(|val| {
                val.map_or_else(
                    |_| None,
                    |(id, trade)| {
                        bincode::deserialize::<Trade>(&trade)
                            .map_or_else(|_| None, |trade| Some(trade))
                    },
                )
            })
            .collect()
    }

    /// Get all queries sent by the user
    pub fn get_all_queries_sent(&self) -> Vec<Query> {
        let mut db = self.db.lock().unwrap();
        db.open_tree(QUERIES_SENT)
            .expect("db: failed to open tree")
            .iter()
            .filter_map(|val| {
                val.map_or_else(
                    |_| None,
                    |(id, query)| {
                        bincode::deserialize::<Query>(&query)
                            .map_or_else(|_| None, |query| Some(query))
                    },
                )
            })
            .collect()
    }

    /// Get all queries received over p2p network
    pub fn get_all_queries_recv(&self) -> Vec<Query> {
        let mut db = self.db.lock().unwrap();
        db.open_tree(QUERIES_RECEIVED)
            .expect("db: failed to open tree")
            .iter()
            .filter_map(|val| {
                val.map_or_else(
                    |_| None,
                    |(id, query)| {
                        bincode::deserialize::<Query>(&query)
                            .map_or_else(|_| None, |query| Some(query))
                    },
                )
            })
            .collect()
    }

    /// Find all bids received for a query by query id
    pub fn find_all_bids_recv_by_query_id(&self, query_id: &QueryId) -> Vec<Bid> {
        let mut db = self.db.lock().unwrap();
        db.open_tree([BIDS_RECEIVED, &query_id.to_be_bytes()].concat())
            .expect("db: failed to open tree")
            .iter()
            .filter_map(|val| {
                val.map_or_else(
                    |_| None,
                    |(id, bid)| {
                        bincode::deserialize::<Bid>(&bid).map_or_else(|_| None, |bid| Some(bid))
                    },
                )
            })
            .collect()
    }

    /// Finds query sent by query id
    pub fn find_query_sent_by_query_id(&self, query_id: &QueryId) -> anyhow::Result<Query> {
        let mut db = self.db.lock().unwrap();
        let queries = db.open_tree(QUERIES_SENT).expect("db: failed to open tree");
        queries
            .iter()
            .find(|res| {
                if let Ok((id, val)) = *res {
                    id == query_id.to_be_bytes()
                } else {
                    false
                }
            })
            .map_or_else(
                || Err(anyhow::anyhow!("Not found")),
                |val| Ok(bincode::deserialize::<Query>(&val?.1)?),
            )
    }

    /// Finds query received by query id
    pub fn find_query_received_by_query_id(&self, query_id: &QueryId) -> anyhow::Result<Query> {
        let mut db = self.db.lock().unwrap();
        let queries = db
            .open_tree(QUERIES_RECEIVED)
            .expect("db: failed to open tree");
        queries
            .iter()
            .find(|res| {
                if let Ok((id, val)) = *res {
                    id == query_id.to_be_bytes()
                } else {
                    false
                }
            })
            .map_or_else(
                || Err(anyhow::anyhow!("Not found")),
                |val| Ok(bincode::deserialize::<Query>(&val?.1)?),
            )
    }

    /// Finds bid sent for a query id
    pub fn find_bid_sent_by_query_id(&self, query_id: &QueryId) -> anyhow::Result<Bid> {
        let mut db = self.db.lock().unwrap();
        let bids = db.open_tree(BIDS_SENT).expect("db: failed to open tree");
        bids.iter()
            .find(|res| {
                if let Ok((id, val)) = *res {
                    id == query_id.to_be_bytes()
                } else {
                    false
                }
            })
            .map_or_else(
                || Err(anyhow::anyhow!("Not found")),
                |val| Ok(bincode::deserialize::<Bid>(&val?.1)?),
            )
    }

    // Find bid received by query id & provider id
    pub fn find_bid_received(
        &self,
        query_id: &QueryId,
        provider_id: &PeerId,
    ) -> anyhow::Result<Bid> {
        let mut db = self.db.lock().unwrap();
        let bids = db
            .open_tree(BIDS_RECEIVED)
            .expect("db: failed to open tree");
        bids.iter()
            .find(|val| {
                if let Ok(flag) = val
                    .map_err(|e| anyhow::Error::from(e))
                    .and_then(|(id, bid)| {
                        bincode::deserialize::<Bid>(&bid)
                            .map_err(|e| anyhow::Error::from(e))
                            .and_then(|bid| {
                                Ok(bid.query_id == *query_id && bid.provider_id == *provider_id)
                            })
                    })
                {
                    flag
                } else {
                    false
                }
            })
            .map_or_else(
                || Err(anyhow::anyhow!("Not Found")),
                |val| Ok(bincode::deserialize::<Bid>(&val?.1)?),
            )
    }

    // Find trade by query id & provider id & requester id
    pub fn find_active_trade(
        &self,
        query_id: &QueryId,
        provider_id: &PeerId,
        requester_id: &PeerId,
    ) -> anyhow::Result<Trade> {
        let mut db = self.db.lock().unwrap();
        let trades = db
            .open_tree(ACTIVE_TRADES)
            .expect("db: failed to open tree");
        trades
            .iter()
            .find(|res| {
                if let Ok(flag) = res
                    .map_err(|e| anyhow::Error::from(e))
                    .and_then(|(id, val)| {
                        bincode::deserialize::<Trade>(&val)
                            .and_then(|trade| {
                                Ok(trade.query_id == *query_id
                                    && trade.bid.provider_id == *provider_id
                                    && trade.query.requester_id == *requester_id)
                            })
                            .map_err(|e| anyhow::Error::from(e))
                    })
                {
                    flag
                } else {
                    false
                }
            })
            .map_or_else(
                || Err(anyhow::anyhow!("Not found")),
                |val| Ok(bincode::deserialize::<Trade>(&val?.1)?),
            )
    }
}
