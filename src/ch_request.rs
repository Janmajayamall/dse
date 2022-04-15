use super::ethnode::{self, EthNode};
use super::network::{self};
use super::network_client;
use super::storage::{self};
use ethers::types::{Address, Signature, H256, U256};
use libp2p::{kad::record, PeerId};
use std::{sync::Arc, time::SystemTime};
use tokio::time;

struct LoadingState {
    pub peers: Vec<PeerId>,
    pub request_count: u32,
    pub loading_in_progress: bool,
    pub timeout: SystemTime,
}

impl LoadingState {
    pub fn query_from(&mut self) -> Option<PeerId> {
        // Each peer is only requested
        // 3 times
        if self.request_count <= 3 {
            self.request_count += 1;
        } else {
            self.request_count = 1;
            self.peers.remove(1);
        }

        self.peers.first().and_then(|v| Some(v.clone()))
    }
}

pub struct ChRequest {
    network_client: network_client::Client,
    storage: Arc<storage::Storage>,
    ethnode: EthNode,
}

impl ChRequest {
    /// Loads commit history of a wallet address
    /// by first finding `providers` for the wallet
    /// on DHT and querying `N` (N = 1 for now)of
    /// them for history.
    pub async fn load_commit_history(&mut self, wallet_address: Address) {
        // find providers on DHT
        match self
            .network_client
            .dht_get_providers(record::Key::new(wallet_address.as_fixed_bytes()))
            .await
        {
            Ok(response) => {
                // find wallet_address's current epoch and owner_address
                let epoch = self.ethnode.get_current_epoch(&wallet_address).await;
                let owner_address = self.ethnode.owner_address(&wallet_address).await;

                // prepare loading state
                let mut loading_state = LoadingState {
                    peers: response.providers.into_iter().collect(),
                    request_count: 0,
                    loading_in_progress: false,
                    timeout: SystemTime::now(),
                };

                if let Ok(mut network_event_receiver) =
                    self.network_client.subscribe_network_events().await
                {
                    let mut interval = time::interval(time::Duration::from_secs(5));

                    loop {
                        tokio::select! {
                            Ok(event) = network_event_receiver.recv() => {
                                match event {
                                    network::NetworkEvent::DseMessageRequestRecv{
                                        sender_peer_id,
                                        request_id,
                                        request
                                    } => {
                                        match request {
                                            network::DseMessageRequest::CommitHistory(network::CommitHistoryRequest::Update{
                                                    wallet_address,
                                                    commits,
                                                    last_batch
                                                }
                                            )=>{
                                                // check that all commits are valid
                                                let commits = commits.into_iter().filter(|c| {
                                                    // TODO add more checks if there exist
                                                    c.epoch == epoch && c.signing_address().map_or_else(|| false, |signature| signature == owner_address)
                                                }).collect();

                                                // FIXME: We might be adding duplicate commits.
                                                // Change this behaviour later
                                                self.storage.add_commits_to_commit_history(&wallet_address, commits);

                                                // reseet timeout
                                                loading_state.timeout = SystemTime::now();

                                                if last_batch {
                                                    return;
                                                }
                                            },
                                            _ => {}
                                        }

                                    },
                                    _ => {

                                    }
                                }

                                // TODO catch necessary network events related to commit history
                                // and add them to storage
                            }
                            _ = interval.tick() => {

                                // If there has been no request for `30 Secs`
                                // then query a new peer.
                                if loading_state.timeout.elapsed().map_or_else(|_| true, |e| e.as_secs() > 30) {
                                    // send a request to one of the peers
                                    if let Some(peer_id) = loading_state.query_from() {
                                        match self.network_client.send_dse_message_request(peer_id, network::DseMessageRequest::CommitHistory(network::CommitHistoryRequest::WantHistory{wallet_address })).await {
                                            Ok(network::DseMessageResponse::Ack) => {
                                                // reset timeout
                                                loading_state.timeout = SystemTime::now();
                                            },
                                            _ => {
                                                // FIXME: Rn we just wait for next 30 secs if peer either fails
                                                // to respond or send a Bad response.
                                                // We might want to change this behaviour.
                                            }
                                        }
                                    }else {
                                        //TODO there are no peers to query from
                                    }
                                }
                            }
                        }
                    }
                } else {
                    // TODO: return with error
                }
            }
            Err(e) => {}
        }

        // start querying one of them for the commit history
    }
}
