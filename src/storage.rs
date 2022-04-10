use bindcode::serialize;
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
    query_id: QueryId,
}

const TRADES_AS_REQUESTER_TREE: &[u8] = b"trades-as-requester";
const TRADES_AS_BIDDER_TREE: &[u8] = b"trades-as-bidder";
const QUERIES_SENT: &[u8] = b"queries-sent";
const QUERIES_RECEIVED: &[u8] = b"queries-received";
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

    /// Creates new Trade with status BidPlaced
    /// for the Query sent
    pub fn add_bid_received_for_query(&self, bid: Bid, query: Query) {
        let mut db = self.db.lock().unwrap();

        // tree that stores all trades related to a query
        // is named as `trades{query_id}`
        let trades = db
            .open_tree([TRADES_AS_REQUESTER_TREE, &query.id.to_be_bytes()].concat())
            .expect("db: failed to open tree");
        let trade = Trade {
            query,
            bid,
            query_id: query.id,
            status: TradeStatus::BidPlaced,
        };

        // BidderId uniquely identifies a bid received for
        // the query sent
        trades.insert(bid.bidder_id.to_bytes(), serialize(&trade).unwrap());
    }

    pub fn add_bid_sent_for_query(&self, bid: Bid, query: Query) {
        let mut db = self.db.lock().unwrap();

        let trades = db
            .open_tree([TRADES_AS_BIDDER_TREE, &query.id.to_be_bytes()].concat())
            .expect("db: failed to open tree");
        let trade = Trade {
            query,
            bid,
            query_id: query.id,
            status: TradeStatus::BidPlaced,
        };

        // QueryId uniquely identifies bid sent for
        // a query received
        trades.insert(query.id.to_be_bytes(), serialize(&trade).unwrap());
    }

    /// Adds new query published by the node
    pub fn add_query_sent(&self, query: Query) {
        let mut db = self.db.lock().unwrap();
        let queries_sent = db
            .open_tree([QUERIES_SENT, &query.id.to_be_bytes()].concat())
            .expect("db: failed to open tree");

        // TODO: check that you are not overwriting
        // previous query with same id.

        queries_sent.insert(query.id.to_be_bytes(), serialize(&query).unwrap());
    }

    /// Adds new query received by the node over gossipsub
    pub fn add_query_received(&self, query: Query) {
        let mut db = self.db.lock().unwrap();
        let queries_sent = db
            .open_tree([QUERIES_RECEIVED, &query.id.to_be_bytes()].concat())
            .expect("db: failed to open tree");

        // TODO: check that you are not overwriting
        // previous query with same id.

        queries_sent.insert(query.id.to_be_bytes(), serialize(&query).unwrap());
    }
}
