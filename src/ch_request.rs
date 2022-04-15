use super::network_client;
use super::storage::{self, TradeStatus};
use ethers::{
    types::{Address, Signature, H256, U256},
    utils::keccak256,
};
use std::sync::Arc;

pub struct ChRequest {
    network_client: network_client::Client,
    storage: Arc<storage::Storage>,
}

impl ChRequest {
    pub async fn load_commit_history(of_address: Address) {
        // find providers on DHT

        // start querying one of them for the commit history
    }
}
