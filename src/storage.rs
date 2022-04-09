use libp2p::{Multiaddr, PeerId};
use sled::{Config, Db};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type QueryId = u32;

/// Stores qeury string and
/// other realted info to the query
struct QueryData {
    query_string: String,
}

struct Query {
    id: QueryId,
    data: QueryData,
    requester_addr: Multiaddr,
    requester_id: PeerId,
}

struct Bid {
    query_id: QueryId,
    bidder_addr: Multiaddr,
    bidder_id: PeerId,
    charge: ethers::types::U256,
}

enum TradeStatus {
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
struct Trade {
    /// Trade query
    query: Query,
    /// Trade bid (accepted by requester)
    bid: Bid,
    /// Status of the trade
    status: TradeStatus,
    query_id: QueryId,
}

pub struct Storage {
    /// keeps track of unused indexes
    unused_indexes: u32,
    /// All trades <QueryId, Trade>
    trades: Mutex<Db>,
    /// All active tradesn (Ones at or before
    /// status CommitsInvalidated)
    active_trades: Mutex<Db>,
}

impl Storage {
    // functions
    // maintain DB for queries sent
    // fn for adding new Trade
    // fn for updating Trade status
}
