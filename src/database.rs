use libp2p::PeerId;
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use sled::{Config, Db};
use std::path::Path;

use super::indexer;

#[derive(Debug, Clone)]
pub struct Database {
    main: Db,
}

impl Database {
    // insert user queries
    // insert received queries
    // insert bid for a usr query

    pub fn insert_user_query(&mut self, query: &indexer::QueryReceived) {
        debug!("inserting user query {:?}", query);

        let user_queries = self
            .main
            .open_tree(b"user_queries")
            .expect("db: failed to read user queries");
        user_queries.insert(query.id.to_be_bytes(), bincode::serialize(&query).unwrap());
    }

    pub fn insert_user_bid_wth_status(&mut self, bid: &indexer::BidReceivedWithStatus) {
        debug!("inserting user bid with status {:?}", bid);
        let user_bids = self
            .main
            .open_tree(b"user_bids")
            .expect("db: failed to read user bids");
        user_bids.insert(
            bid.bid_recv.query_id.to_be_bytes(),
            bincode::serialize(&bid).unwrap(),
        );
    }

    pub fn insert_received_query(&mut self, query: &indexer::QueryReceived) {
        debug!("inserting received query {:?}", query);
        let user_queries = self
            .main
            .open_tree(b"user_received_queries")
            .expect("db: failed to read user received queries");
        user_queries.insert(query.id.to_be_bytes(), bincode::serialize(&query).unwrap());
    }

    pub fn insert_received_bid_with_status(&mut self, bid: &indexer::BidReceivedWithStatus) {
        debug!("inserting received bid {:?}", bid);
        let query_bids = self
            .main
            .open_tree(bid.bid_recv.query_id.to_be_bytes())
            .expect("db: failed to read query bids");
        query_bids.insert(
            bid.bid_recv.bidder_id.to_bytes(),
            bincode::serialize(&bid).unwrap(),
        );
    }

    pub fn find_user_bids_with_status(
        &self,
    ) -> Result<Vec<indexer::BidReceivedWithStatus>, anyhow::Error> {
        let mut all_bids = Vec::<indexer::BidReceivedWithStatus>::new();
        let user_bids = self.main.open_tree(b"user_bids")?;
        for val in &user_bids {
            let (_, bid) = val?;
            all_bids.push(bincode::deserialize::<indexer::BidReceivedWithStatus>(&bid).unwrap());
        }
        Ok(all_bids)
    }

    pub fn find_user_bid_with_status(
        &self,
        query_id: &indexer::QueryId,
    ) -> Option<indexer::BidReceivedWithStatus> {
        match self.find_user_bids_with_status() {
            Ok(mut bids) => bids.into_iter().find(|b| b.bid_recv.query_id == *query_id),
            Err(e) => None,
        }
    }

    pub fn find_user_queries(&self) -> Result<Vec<indexer::QueryReceived>, anyhow::Error> {
        let mut all_queries: Vec<indexer::QueryReceived> = Vec::new();
        let user_queries = self.main.open_tree(b"user_queries")?;
        for val in &user_queries {
            let (id, query) = val?;
            let query = bincode::deserialize::<indexer::QueryReceived>(&query).unwrap();
            all_queries.push(query);
        }
        Ok(all_queries)
    }

    pub fn find_recv_queries(&self) -> Result<Vec<indexer::QueryReceived>, anyhow::Error> {
        let mut all_queries = Vec::<indexer::QueryReceived>::new();
        let queries = self.main.open_tree(b"user_received_queries")?;
        for val in &queries {
            let (id, query) = val?;
            all_queries.push(bincode::deserialize::<indexer::QueryReceived>(&query).unwrap());
        }
        Ok(all_queries)
    }

    pub fn find_query_bids_with_status(
        &self,
        query_id: &indexer::QueryId,
    ) -> Result<Vec<indexer::BidReceivedWithStatus>, anyhow::Error> {
        let mut all_bids = Vec::<indexer::BidReceivedWithStatus>::new();
        let bids = self.main.open_tree(query_id.to_be_bytes())?;
        for val in &bids {
            let (id, bid) = val?;
            all_bids.push(bincode::deserialize::<indexer::BidReceivedWithStatus>(&bid).unwrap());
        }
        Ok(all_bids)
    }

    pub fn find_query_bid_with_status(
        &self,
        query_id: &indexer::QueryId,
        bidder_id: &PeerId,
    ) -> Option<indexer::BidReceivedWithStatus> {
        match self.find_query_bids_with_status(query_id) {
            Ok(mut bids) => bids
                .into_iter()
                .find(|b| b.bid_recv.bidder_id == *bidder_id),
            Err(e) => None,
        }
    }
}

pub fn new(node_id: String) -> Database {
    let mut path = "./dbs/".to_string();
    path.push_str(&node_id);
    let config = Config::new().path(path);
    Database {
        main: config.open().expect("db: Failed to open db"),
    }
}
