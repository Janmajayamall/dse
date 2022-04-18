use ethers::{
    types::{Address, Signature, H256, U256},
    utils::keccak256,
};
use libp2p::{Multiaddr, PeerId};
use log::debug;
use log::error;
use rand::Rng;
use serde::{Deserialize, Serialize};
use sled::{Config, Db};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    time::SystemTime,
};

pub type QueryId = u32;

use super::network_client;

/// Commit history
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct CommitHistory {
    /// Wallet address that made these commits
    pub wallet_address: Address,
    /// All commits
    pub commits: Vec<Commit>,
}

impl CommitHistory {
    pub fn update_invalditing_signature(&mut self, id: u32, invalidating_signature: Signature) {
        // TODO: Check that signature is valid for the given id

        // TODO: Iterate thru all commits and add invalidating signature
        // to commits corresponding to the given ID.

        for c in self.commits.iter_mut() {
            if c.u == id {
                c.invalidating_signature = Some(invalidating_signature);
            }
        }
    }

    pub fn are_indexes_in_conflict(&self, indexes: &Vec<u32>) {
        self.commits.iter().find(|c| {
            if c.c_type == CommitType::T1 && c.invalidating_signature != None {
                // c_type T1 is invalidated with
                // invalidating_signature
                // Note that invalidating signature is validated
                // before being added, so no need to validate it here
                // again.
                false
            } else {
                let mut flag = false;
                'outer: for i in (1..c.indexes.len()).step_by(2) {
                    for j in 1..indexes.len() {
                        if indexes[j] >= c.indexes[i - 1] && indexes[j - 1] <= c.indexes[i] {
                            flag = true;
                            break 'outer;
                        }
                    }
                }
                flag
            }
        });
    }

    pub fn add_new_commits(&mut self, mut commits: Vec<Commit>) {
        self.commits.append(&mut commits);
    }
}

/// Stores qeury string and
/// other realted info to the query
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct QueryData {
    query_string: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Query {
    pub id: QueryId,
    pub data: QueryData,
    pub requester_addr: Multiaddr,
    pub requester_id: PeerId,
    // FIXME: I don't think this is needed since
    // Commit carries corresponding wallet address
    pub requester_wallet_address: Address,
}

impl Query {
    pub async fn from_data(
        data: QueryData,
        node_id: PeerId,
        mut network_client: network_client::Client,
        wallet_address: Address,
    ) -> anyhow::Result<Self> {
        // id = keccack256({node_id}+{UNIX time in secs})[32 bits]
        // let mut id = node_id.to_bytes();
        // id.append(
        //     &mut SystemTime::now()
        //         .duration_since(SystemTime::UNIX_EPOCH)?
        //         .as_secs()
        //         .to_le_bytes()
        //         .to_vec(),
        // );
        // let id = ethers::utils::keccak256(&id);
        // let id = u32::from_be_bytes(id[..4].try_into().unwrap());
        //
        // OR just random
        let id = rand::thread_rng().gen::<u32>();

        network_client
            .network_details()
            .await
            .map(|(requester_id, requester_addr)| Query {
                id: 1, // FIXME: this is just for debugging
                data,
                requester_id,
                requester_addr,
                requester_wallet_address: wallet_address,
            })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BidData {
    query_id: QueryId,
    charge: ethers::types::U256,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeQuote {
    pub charge: ethers::types::U256,
    pub id: u32,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct Bid {
    pub query_id: QueryId,
    pub provider_addr: Multiaddr,
    pub provider_id: PeerId,
    // FIXME: I don't think this is needed since
    // Commit carries corresponding wallet address
    pub provider_wallet_address: Address,
    pub charge: U256,
}

impl Bid {
    pub async fn from_data(
        data: BidData,
        mut network_client: network_client::Client,
        wallet_address: Address,
    ) -> anyhow::Result<Self> {
        network_client
            .network_details()
            .await
            .map(|(provider_id, provider_addr)| Bid {
                query_id: data.query_id,
                provider_id,
                provider_addr,
                charge: data.charge,
                provider_wallet_address: wallet_address,
            })
    }
}

#[derive(Deserialize, Serialize, Debug, PartialEq, Clone)]
pub enum TradeStatus {
    /// Provider should send StartCommit
    /// /// FIXME: Not needed anymore
    PSendStartCommit,
    /// Requester is waiting for Start Commit
    /// from provider
    /// FIXME: Not needed anymore
    WaitingStartCommit,
    /// Provider is waiting for Requester's T1
    /// Commit
    WaitingRT1Commit,
    /// Requester should send T1 commit
    RSendT1Commit,
    /// Provider is processing Requester's T1
    /// commit
    ProcessingRT1Commit,
    /// Requester is waiting for Provider's T1
    /// Commit
    WaitingPT1Commit,
    /// Provider should send T1 commit
    PSendT1Commit,
    /// Requester is processing Provider's T1
    /// commit
    ProcessingPT1Commit,
    /// Provider is waiting for Requester's T2
    /// Commit
    WaitingRT2Commit,
    /// Requester should send T2 commit
    RSendT2Commit,
    /// Provider is processing Requester's T2
    /// commit
    ProcessingRT2Commit,
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
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Trade {
    pub id: u32,

    pub requester_addr: Multiaddr,
    pub requester_id: PeerId,

    pub provider_addr: Multiaddr,
    pub provider_id: PeerId,

    pub charge: U256,

    /// Status of the trade
    pub status: TradeStatus,
    /// Flag whether node is requester
    /// OR provider
    pub is_requester: bool,
}

impl Trade {
    pub fn t1CommitAmount(&self) -> U256 {
        self.charge.div_mod(U256::from_dec_str("2").unwrap()).0
    }

    pub fn t2CommitAmount(&self) -> U256 {
        self.charge
    }

    pub fn update_status(&mut self, to: TradeStatus) {
        debug!(
            "Updated trade status for id {} from {:?} to {:?}",
            self.id, self.status, to
        );
        self.status = to;
    }

    pub fn counter_party_details(&self) -> (PeerId, Multiaddr) {
        if self.is_requester {
            (self.provider_id, self.provider_addr)
        } else {
            (self.requester_id, self.requester_addr)
        }
    }

    /// Returns true if trade status is SOMETHING
    /// send that needs to be triggered automatically,
    /// by indexer otherwise returns false.
    pub fn is_sending_status(&self) -> bool {
        self.status == TradeStatus::RSendT1Commit
            || self.status == TradeStatus::PSendT1Commit
            || self.status == TradeStatus::RSendT2Commit
    }

    pub async fn from_quote(
        quote: TradeQuote,
        i_want: IWant,
        status: TradeStatus,
        mut network_client: network_client::Client,
        is_requester: bool,
    ) -> anyhow::Result<Self> {
        network_client
            .network_details()
            .await
            .map(|(node_id, node_addr)| Trade {
                id: quote.id,
                requester_addr: i_want.requester_addr,
                requester_id: i_want.requester_id,
                provider_addr: node_addr,
                provider_id: node_id,
                charge: quote.charge,
                status,
                is_requester,
            })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub enum CommitType {
    /// Commitment Type 1
    T1 = 1,
    /// Commitment Type 2
    T2 = 2,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Commit {
    /// Index ranges used for commitment.
    ///
    /// There can one or more index ranges. Every index range
    /// is defined by i & i + 1 element. Therefore, length of indexes
    /// vec should atleast be 2 (i.e. At least one index range). Also,
    /// length indexes cannot be odd.
    pub indexes: Vec<u32>,
    /// epoch during which the commitment is valid
    pub epoch: U256,
    /// unique id that identifies commitment with the
    /// corresponding query id
    pub u: u32,
    /// commit type 1 or 2
    pub c_type: CommitType,
    /// address that invalidates the commitment
    pub i_address: Address,
    /// address that can redeeem this commitment
    /// with invalidating signature.
    /// Only needed for c_type = 2
    pub r_address: Address,
    /// Wallet address to which this time-locked
    /// commitment belongs
    pub wallet_address: Address,
    /// owner's signature on commitment's blob
    pub signature: Option<Signature>,
    /// invalidting signature for the commit
    pub invalidating_signature: Option<Signature>,
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

    pub fn signing_address(&self) -> Option<Address> {
        self.signature.and_then(|signature| {
            signature
                .recover(self.commit_hash())
                .map_or_else(|_| None, |s| Some(s))
        })
    }

    /// Validates received commitment.
    /// Returns true if valid, otherwise false.
    ///
    /// > **Note**
    /// > Call this before transitioning from SOMETHING
    /// > Waiting to SOMETHING Processing
    pub fn is_commit_valid(&self, trade: &Trade, owner_address: &Address) -> bool {
        // 2 consecutive values define 1 index range,
        // thus indexes can only be in increasing order.
        for i in 1..self.indexes.len() {
            if self.indexes[i] < self.indexes[i - 1] {
                return false;
            }
        }

        // Check signature
        if self
            .signing_address()
            .map_or_else(|| true, |a| a != *owner_address)
        {
            return false;
        }

        // FIXME: Add validations
        match trade.status {
            TradeStatus::WaitingRT1Commit => true,
            TradeStatus::WaitingPT1Commit => true,
            TradeStatus::WaitingRT2Commit => true,
            _ => false,
        }
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

const COMMIT_HISTORY: &[u8] = b"commit-history";

const START_COMMITS: &[u8] = b"start-commits";
const I_WANTS: &[u8] = b"i-wants";

pub struct Storage {
    /// Stores all graphs
    ///
    /// Graphs include - AllTrades, ActiveTrades
    /// QueriesSent
    db: Mutex<Db>,
    /// ID counter of clients
    client_id_counter: AtomicUsize,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct IWant {
    requester_id: PeerId,
    requester_addr: Multiaddr,
    id: u32,
}

impl Storage {
    pub fn new(node_id: PeerId) -> Self {
        let mut path = "./dbs/".to_string();
        path.push_str(&node_id.to_base58());
        let config = Config::new().path(path);

        Self {
            db: Mutex::new(config.open().expect("Failed to open main db")),
            client_id_counter: AtomicUsize::new(1),
        }
    }

    /// Stores newly received `IWANT` messge from requester
    pub fn add_i_want(&self, msg: &IWant) {
        let mut db = self.db.lock().unwrap();
        let tree = db.open_tree(I_WANTS).expect("db: failed to open tree");
        tree.insert(msg.id.to_be_bytes(), bincode::serialize(&msg).unwrap());
    }

    /// Stores `Trade`  received from provider
    pub fn add_active_trade(&self, msg: &Trade) {
        let mut db = self.db.lock().unwrap();

        let tree = db
            .open_tree(ACTIVE_TRADES)
            .expect("db: failed to open tree");

        tree.insert(msg.id.to_be_bytes(), bincode::serialize(&msg).unwrap());
    }

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

    /// Stores new trade
    pub fn add_new_trade(&self, trade: Trade) {
        let db = self.db.lock().unwrap();
        let trades = db
            .open_tree(ACTIVE_TRADES)
            .expect("db: failed to open tree");
        trades.insert(trade.id.to_be_bytes(), bincode::serialize(&trade).unwrap());
    }

    /// Adds new commit to the commit history
    /// of a wallet addess
    pub fn add_commits_to_commit_history(&self, wallet_address: &Address, commits: Vec<Commit>) {
        let mut existing_h = {
            if let Some(h) = self.find_commit_history(wallet_address) {
                h
            } else {
                CommitHistory {
                    wallet_address: *wallet_address,
                    commits: Default::default(),
                }
            }
        };

        // add new commit to exisiting commits
        existing_h.add_new_commits(commits);

        let mut db = self.db.lock().unwrap();
        let commit_history = db
            .open_tree(COMMIT_HISTORY)
            .expect("db: failed to open tree")
            .insert(
                wallet_address.as_bytes(),
                bincode::serialize(&existing_h).unwrap(),
            );
    }

    /// Updates Active Trade
    ///
    /// Returns err if Trade does not pre-exists.
    pub fn update_active_trade(&self, trade: Trade) -> anyhow::Result<()> {
        self.find_active_trade(&trade.id).and_then(|_| {
            let mut db = self.db.lock().unwrap();
            db.open_tree(ACTIVE_TRADES)
                .expect("db: failed to open tree")
                .insert(trade.id.to_be_bytes(), bincode::serialize(&trade).unwrap())
                .map(|_| ())
                .map_err(anyhow::Error::from)
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

    /// Find commit history of `wallet_address`
    pub fn find_commit_history(&self, wallet_address: &Address) -> Option<CommitHistory> {
        let mut db = self.db.lock().unwrap();
        db.open_tree(COMMIT_HISTORY)
            .expect("db: failed to open tree")
            .iter()
            .find(|val| {
                if let Ok(Ok(flag)) = val.map(|(_, cm)| {
                    bincode::deserialize::<CommitHistory>(&cm)
                        .map(|cm| cm.wallet_address == *wallet_address)
                }) {
                    flag
                } else {
                    false
                }
            })
            .and_then(|val| Some(bincode::deserialize::<CommitHistory>(&val.unwrap().1).unwrap()))
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
                if let Ok((id, val)) = res {
                    *id == query_id.to_be_bytes()
                } else {
                    false
                }
            })
            .map_or_else(
                || Err(anyhow::anyhow!("Not found")),
                |val| Ok(bincode::deserialize::<Query>(&val?.1)?),
            )
    }

    /// Finds I wants
    pub fn find_i_want(&self, iid: &u32) -> Option<IWant> {
        let mut db = self.db.lock().unwrap();
        let tree = db.open_tree(I_WANTS).expect("db: failed to open tree");
        tree.iter()
            .find(|res| {
                if let Ok((id, val)) = res {
                    *id == iid.to_be_bytes()
                } else {
                    false
                }
            })
            .map_or_else(
                || None,
                |val| Some(bincode::deserialize::<IWant>(&val.unwrap().1).unwrap()),
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
                if let Ok((id, val)) = res {
                    *id == query_id.to_be_bytes()
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
                if let Ok((id, val)) = res {
                    *id == query_id.to_be_bytes()
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
        let db = self.db.lock().unwrap();
        let bids = db
            .open_tree([BIDS_RECEIVED, &query_id.to_be_bytes()].concat())
            .expect("db: failed to open tree");
        bids.iter()
            .find(|val| {
                if let Ok(Ok(flag)) = val.to_owned().map_err(anyhow::Error::from).map(|(_, bid)| {
                    bincode::deserialize::<Bid>(&bid)
                        .map(|bid| bid.query_id == *query_id && bid.provider_id == *provider_id)
                }) {
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

    /// Find trade by query id & provider id & requester id
    pub fn find_active_trade(&self, iid: &u32) -> anyhow::Result<Trade> {
        let mut db = self.db.lock().unwrap();
        let trades = db
            .open_tree(ACTIVE_TRADES)
            .expect("db: failed to open tree");
        trades
            .iter()
            .find(|res| {
                if let Ok((id, _)) = res {
                    *id == iid.to_be_bytes()
                } else {
                    false
                }
            })
            .map_or_else(
                || Err(anyhow::anyhow!("Not found")),
                |val| Ok(bincode::deserialize::<Trade>(&val?.1)?),
            )
    }

    pub fn next_client_id(&self) -> usize {
        self.client_id_counter.fetch_add(1, Ordering::Relaxed)
    }
}
