use super::network_client;
use super::storage::{self, TradeStatus};
use ethers::{
    types::{Address, Signature, H256, U256},
    utils::keccak256,
};
use libp2p::{kad::record, PeerId};
use std::sync::Arc;

pub struct ChRequest {
    network_client: network_client::Client,
    storage: Arc<storage::Storage>,
}

impl ChRequest {
    /// Loads commit history of a wallet address
    /// by first finding `providers` for the wallet
    /// on DHT and querying `N` (N = 1 for now)of
    /// them for history.
    pub async fn load_commit_history(&self, wallet_address: Address) {
        // find providers on DHT
        match self
            .network_client
            .dht_get_providers(record::Key::new(wallet_address.as_fixed_bytes()))
            .await
        {
            Ok(response) => {
                // TODO start_loading_from_peer with one of the peers
            }
            Err(e) => {}
        }

        // start querying one of them for the commit history
    }

    pub async fn start_loading_from_peer(&self, peer_id: &PeerId, walllet_address: Address) {
        if let Ok(mut network_event_receiver) = self.network_client.subscribe_network_events().await
        {
            loop {
                tokio::select! {
                    event = network_event_receiver.recv() => {
                        // TODO catch necessary network events related to commit history
                        // and add them to storage
                    }
                }
            }
        } else {
            // TODO: return with error
        }
    }
}
