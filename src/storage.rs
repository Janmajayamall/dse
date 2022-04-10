use libp2p::{Multiaddr, PeerId};
use serde::{Deserialize, Serialize};
use sled::{Config, Db};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type QueryId = u32;

/// Stores qeury string and
/// other realted info to the query
#[derive(Deserialize, Serialize, Debug)]
pub struct QueryData {
    query_string: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct Query {
    id: QueryId,
    data: QueryData,
    requester_addr: Multiaddr,
    requester_id: PeerId,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct Bid {
    query_id: QueryId,
    bidder_addr: Multiaddr,
    bidder_id: PeerId,
    charge: ethers::types::U256,
}

#[derive(Deserialize, Serialize, Debug)]
pub enum TradeStatus {
    /// Bid has been sent/received
    BidPlaced,
    /// Requester has accepted the bid
    BidAccepted,
    /// Requester has committed T1 commits
    ReqT1Committed,
    /// Bidder has committed T1 commits
    BidT1Committed,
    /// Requester has committed T2 commits
    ReqT2Committed,
    /// Bidder has provided the service
    ServiceProvided,
    /// Requester has shared the invalidating signature with
    /// bidder
    CommitsInvalidated,
}

/// Stores trade infor between
/// node and some peer
#[derive(Deserialize, Serialize, Debug)]
pub struct Trade {
    /// Trade query
    query: Query,
    /// Trade bid (accepted by requester)
    bid: Bid,
    /// Status of the trade
    status: TradeStatus,
    /// Query id
    query_id: QueryId,
    /// Flag whether node is requester
    /// OR provider
    is_requester: bool,
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
        bids.insert(bid.bidder_id.to_bytes(), bincode::serialize(&bid).unwrap());
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
        let queries_sent = db.open_tree(QUERIES_SENT).expect("db: failed to open tree");

        // TODO: check that you are not overwriting
        // previous query with same id.

        queries_sent.insert(query.id.to_be_bytes(), bincode::serialize(&query).unwrap());
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
            status: TradeStatus::BidAccepted,
            query_id: query.id,
            is_requester,
        };
        trades.insert(query.id.to_be_bytes(), bincode::serialize(&trade).unwrap());
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
                                    && trade.bid.bidder_id == *provider_id
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
