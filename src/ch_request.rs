use super::network_client;
use super::storage::{self};
use crate::ethnode::EthNode;
use crate::network::{CommitRequest, CommitResponse, NetworkEvent};
use ethers::types::Address;
use libp2p::{kad::record, PeerId};
use log::{debug, error};
use std::{sync::Arc, time::SystemTime};
use tokio::time;

#[derive(Clone, PartialEq)]
enum State {
    Pending,
    InProgress,
    Ended,
}

struct LoadingState {
    pub peers: Vec<PeerId>,
    pub request_count: u32,
    pub loading_in_progress: bool,
    pub timeout: SystemTime,
    pub state: State,
}

impl LoadingState {
    pub fn query_from(&mut self) -> Option<PeerId> {
        // Each peer is only requested
        // 3 times
        if self.request_count <= 3 {
            self.request_count += 1;
        } else {
            self.request_count = 1;
            self.peers.remove(0);
        }

        self.peers.first().and_then(|v| Some(v.clone()))
    }
}

pub struct ChRequest {
    pub network_client: network_client::Client,
    pub storage: Arc<storage::Storage>,
    pub ethnode: EthNode,
}

impl ChRequest {
    /// Loads commit history of a wallet address
    /// by first finding `providers` for the wallet
    /// on DHT and querying `N` (N = 1 for now) of
    /// them for history.
    pub async fn load_commit_history(&mut self, wallet_address: &Address) -> anyhow::Result<()> {
        // find providers on DHT
        match self
            .network_client
            .dht_get_providers(record::Key::new(wallet_address.as_fixed_bytes()))
            .await
        {
            Ok(response) => {
                let peers = response.providers.into_iter().collect();
                debug!(
                    "Found providers {:?} for wallet_address {}",
                    peers, wallet_address
                );

                // find wallet_address's current epoch and owner_address
                let epoch = self.ethnode.get_current_epoch(&wallet_address).await;
                let owner_address = self.ethnode.owner_address(&wallet_address).await;

                // prepare loading state
                let mut loading_state = LoadingState {
                    peers,
                    request_count: 0,
                    loading_in_progress: false,
                    timeout: SystemTime::now(),
                    state: State::Pending,
                };

                if let Ok(mut network_event_receiver) =
                    self.network_client.subscribe_network_events().await
                {
                    let mut interval = time::interval(time::Duration::from_secs(10));

                    loop {
                        tokio::select! {
                            Ok(event) = network_event_receiver.recv() => {
                                match event {
                                    NetworkEvent::CommitRequest{
                                        sender_peer_id,
                                        request_id,
                                        request: CommitRequest::Update {
                                            wallet_address,
                                            commits,
                                            last_batch,
                                        }
                                    } => {
                                        debug!("Received commit history for wallet address {}", wallet_address);

                                        // check that all commits are valid
                                        let commits = commits.into_iter().filter(|c| {
                                            // TODO add more checks if there exist
                                            c.epoch == epoch && c.signing_address().map_or_else(|| false, |signature| signature == owner_address)
                                        }).collect();

                                        // FIXME: We might be adding duplicate commits.
                                        // Change this behaviour later. Also we haven't
                                        // checked commits for whether signatures are valid.
                                        self.storage.add_commits_to_commit_history(&wallet_address, commits);

                                        // reseet timeout
                                        loading_state.timeout = SystemTime::now();

                                        if last_batch {
                                            loading_state.state = State::Ended;
                                        }
                                    }
                                    _ => {}
                                }

                            }
                            _ = interval.tick() => {
                                if loading_state.state == State::Ended {
                                    // loaded commit history of wallet
                                    return Ok(());
                                }else {
                                    // If there has been no request for `30 Secs`
                                    // then query a new peer.
                                    if  loading_state.state == State::Pending || loading_state.timeout.elapsed().map_or_else(|_| true, |e| e.as_secs() > 30) {
                                        // send a request to one of the peers
                                        if let Some(peer_id) = loading_state.query_from() {
                                            match self.network_client.send_commit_request(peer_id, CommitRequest::WantHistory{wallet_address:*wallet_address}).await {
                                                Ok(CommitResponse::Ack) => {
                                                    // reset timeout
                                                    loading_state.timeout = SystemTime::now();
                                                    loading_state.state = State::InProgress;
                                                },
                                                _ => {
                                                    // FIXME: Rn we just wait for next `interval` if peer either fails
                                                    // to respond or sends a Bad response.
                                                    // We might want to change this behaviour.
                                                }
                                            }

                                        }else {
                                            debug!("No more peers left to query commit hisotry from for wallet address {}", wallet_address);
                                            // Probably means that means wallet hasn't made pervious commitments.
                                            // resolve with Ok
                                            return Ok(());
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else {
                    error!("failed to subscribes to networke events!");
                    Err(anyhow::anyhow!("Failed!"))
                }
            }
            Err(e) => {
                error!(
                    "Dht get providers failed for wallet address {}",
                    wallet_address
                );
                Err(anyhow::anyhow!("Failed!"))
            }
        }
    }
}
