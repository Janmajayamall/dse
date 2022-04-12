use async_std::channel;
use libp2p::{identity::Keypair, request_response::RequestId, Multiaddr, PeerId};
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tokio::{select, time};

use crate::storage::QueryId;

use super::commit_procedure;
use super::ethnode;
use super::network;
use super::network_client;
use super::server;
use super::storage;
/// Main interface thru which user interacts.
/// That means sends and receives querues & bids.
pub struct Indexer {
    keypair: Keypair,
    /// Sends events to server clients over websocket
    server_client_senders: HashMap<usize, mpsc::UnboundedSender<server::SendWssMessage>>,
    /// network client
    network_client: network_client::Client,
    network_event_receiver: channel::Receiver<network::NetworkEvent>,
    /// global database
    storage: Arc<storage::Storage>,
    ethnode: ethnode::EthNode,

    // receiver of commit procedure events
    commit_proc_event_receiver: channel::Receiver<commit_procedure::CommitProcedureEvent>,
    // sender for commit procedure events
    commit_proc_event_sender: channel::Sender<commit_procedure::CommitProcedureEvent>,

    // query ids of trades in send status currently
    // with active send() CommitProcedure
    active_send_commit_proc: HashSet<QueryId>,
}

impl Indexer {
    pub fn new(
        keypair: Keypair,
        network_client: network_client::Client,
        network_event_receiver: channel::Receiver<network::NetworkEvent>,
        storage: Arc<storage::Storage>,
        ethnode: ethnode::EthNode,
    ) -> Self {
        let (commit_proc_event_sender, commit_proc_event_receiver) = channel::unbounded();

        Self {
            keypair,
            server_client_senders: Default::default(),

            network_client,
            network_event_receiver,

            storage,
            ethnode,

            commit_proc_event_receiver,
            commit_proc_event_sender,

            active_send_commit_proc: Default::default(),
        }
    }

    pub async fn run(mut self) {
        let mut interval = time::interval(time::Duration::from_secs(5));
        loop {
            select! {
                event = self.network_event_receiver.recv() => {
                    if let Ok(event) = event {self.handle_network_event(event).await;}
                },
                _ = interval.tick() => {
                    // get all active trades
                    for trade in self.storage.get_active_trades() {
                        if trade.is_sending_status() && !self.active_send_commit_proc.contains(&trade.query_id) {
                            self.active_send_commit_proc.insert(trade.query_id);

                            // start send() commit procedure
                            let proc = commit_procedure::CommitProcedure::new(
                                self.storage.clone(),
                                trade,
                                self.network_client.clone(),
                                self.ethnode.clone(),
                                self.commit_proc_event_sender.clone()
                            );
                            tokio::spawn(async move {
                                proc.send().await;
                            });
                        }
                    }

                },
                Ok(event) = self.commit_proc_event_receiver.recv() => {
                    use commit_procedure::CommitProcedureEvent;
                    match event {
                        CommitProcedureEvent::SendSuccess{ query_id } => {
                            self.active_send_commit_proc.remove(&query_id);
                        },
                        CommitProcedureEvent::SendFailed{ query_id } => {
                            self.active_send_commit_proc.remove(&query_id);
                        }
                    };
                }
            }
        }
    }

    pub async fn handle_network_event(&self, event: network::NetworkEvent) {
        use network::{DseMessageRequest, DseMessageResponse, GossipsubMessage, NetworkEvent};
        use storage::TradeStatus;
        match event {
            NetworkEvent::GossipsubMessageRecv(GossipsubMessage::NewQuery(query)) => {
                self.storage.add_query_received(query);
                // TODO inform client over WSS
            }
            NetworkEvent::DseMessageRequestRecv {
                sender_peer_id,
                request_id,
                request,
            } => {
                match request {
                    DseMessageRequest::PlaceBid { query_id, bid } => {
                        // Received PlaceBid request from Requester for placing a bid
                        // for a query. Therefore, first check whether node (i.e. Requester)
                        // sent a query with given query id.
                        if let Ok(_) = self
                            .storage
                            .find_query_sent_by_query_id(&query_id)
                            .and_then(|q| {
                                // provider_id should match sender_peer_id
                                // from whom request was received
                                if bid.provider_id == sender_peer_id {
                                    Ok(q)
                                } else {
                                    Err(anyhow::anyhow!("Peer id mismatch"))
                                }
                            })
                        {
                            self.storage.add_bid_received_for_query(&query_id, bid);
                            send_dse_response(
                                request_id,
                                self.network_client.clone(),
                                DseMessageResponse::Ack,
                            );

                            // TODO inform the clients over WSS
                        } else {
                            send_dse_response(
                                request_id,
                                self.network_client.clone(),
                                DseMessageResponse::Bad,
                            );
                        }
                    }
                    DseMessageRequest::AcceptBid { query_id } => {
                        // Received AcceptBid from Requester for bid placed by Node
                        // (i.e. Provider) on their query with given query id.
                        // Therefore, first check that bid was placed & query was received by
                        // the Node.
                        if let Ok((bid, query)) = self
                            .storage
                            .find_bid_sent_by_query_id(&query_id)
                            .and_then(|bid| {
                                self.storage
                                    .find_query_received_by_query_id(&query_id)
                                    .and_then(|query| {
                                        // Check that sender is the requester of query
                                        if query.requester_id == sender_peer_id {
                                            Ok((bid, query))
                                        } else {
                                            Err(anyhow::anyhow!("Peer id mismatch"))
                                        }
                                    })
                            })
                        {
                            // is_requester = false, since node is provider
                            self.storage.add_new_trade(query, bid, false);

                            send_dse_response(
                                request_id,
                                self.network_client.clone(),
                                DseMessageResponse::Ack,
                            );

                            // TODO inform clients over WSS
                        } else {
                            send_dse_response(
                                request_id,
                                self.network_client.clone(),
                                DseMessageResponse::Bad,
                            );
                            // TODO send bad response
                        }
                    }
                    DseMessageRequest::StartCommit { query_id } => {
                        // Received StartCommit from Provider for AcceptedBid on a published
                        // Query by the Node (i.e. Requester). Thus, Trade object should exist
                        // for the given query id with provider id as peer id.
                        if let Ok(mut trade) = self
                            .storage
                            .find_active_trade(
                                &query_id,
                                // request sender should be the provider in the trade
                                &sender_peer_id,
                                &self.keypair.public().to_peer_id(),
                            )
                            .and_then(|trade| {
                                if trade.status == TradeStatus::WaitingStartCommit {
                                    Ok(trade)
                                } else {
                                    Err(anyhow::anyhow!("Invalid request"))
                                }
                            })
                        {
                            // update waiting status to RSendT1Commit
                            trade.update_status(TradeStatus::RSendT1Commit);
                            self.storage.update_active_trade(trade);

                            send_dse_response(
                                request_id,
                                self.network_client.clone(),
                                DseMessageResponse::Ack,
                            );
                        } else {
                            send_dse_response(
                                request_id,
                                self.network_client.clone(),
                                DseMessageResponse::Bad,
                            );
                        }
                    }
                    DseMessageRequest::T1RequesterCommit { query_id, commit } => {
                        // Received T1 Commit from the requester. That means Trade with Node as
                        // Provider and Peer as Requester exists, and TradeStatus is WaitingRT1Commit
                        // Note - This request is only valid if Provider's Trade status is WaitingRT1Commit
                        if let Ok(trade) = self
                            .storage
                            .find_active_trade(
                                &query_id,
                                &self.keypair.public().to_peer_id(),
                                // requester sender should be requester in the trade
                                &sender_peer_id,
                            )
                            .and_then(|trade| {
                                if trade.status == TradeStatus::WaitingRT1Commit {
                                    Ok(trade)
                                } else {
                                    Err(anyhow::anyhow!("Invlaid request"))
                                }
                            })
                        {
                            // In verify fn of commit procedure
                            // trade status is immediately changed to suitable
                            // processing status, thus preventing calling of this
                            // function twice.
                            let proc = commit_procedure::CommitProcedure::new(
                                self.storage.clone(),
                                trade,
                                self.network_client.clone(),
                                self.ethnode.clone(),
                                self.commit_proc_event_sender.clone(),
                            );
                            tokio::spawn(async move {
                                proc.verify(commit).await;
                            });

                            send_dse_response(
                                request_id,
                                self.network_client.clone(),
                                DseMessageResponse::Ack,
                            );
                        } else {
                            send_dse_response(
                                request_id,
                                self.network_client.clone(),
                                DseMessageResponse::Bad,
                            );
                        }
                    }
                    DseMessageRequest::T1ProviderCommit { query_id, commit } => {
                        // Received T1 Commit from provider. Therefore, TradeStatus of the node
                        // (i.e. Requester) should be WaitingPT1Commit
                        if let Ok(trade) = self
                            .storage
                            .find_active_trade(
                                &query_id,
                                // request sender is provider
                                &sender_peer_id,
                                &self.keypair.public().to_peer_id(),
                            )
                            .and_then(|trade| {
                                if trade.status == TradeStatus::WaitingPT1Commit {
                                    Ok(trade)
                                } else {
                                    Err(anyhow::anyhow!("Invlaid request"))
                                }
                            })
                        {
                            let proc = commit_procedure::CommitProcedure::new(
                                self.storage.clone(),
                                trade,
                                self.network_client.clone(),
                                self.ethnode.clone(),
                                self.commit_proc_event_sender.clone(),
                            );
                            tokio::spawn(async move {
                                proc.verify(commit).await;
                            });

                            send_dse_response(
                                request_id,
                                self.network_client.clone(),
                                DseMessageResponse::Ack,
                            );
                        } else {
                            send_dse_response(
                                request_id,
                                self.network_client.clone(),
                                DseMessageResponse::Bad,
                            );
                        }
                    }
                    DseMessageRequest::T2RequesterCommit { query_id, commit } => {
                        // Received T2 Commit from requester.
                        if let Ok(trade) = self
                            .storage
                            .find_active_trade(
                                &query_id,
                                &self.keypair.public().to_peer_id(),
                                // request sender should be requester
                                &sender_peer_id,
                            )
                            .and_then(|trade| {
                                if trade.status == TradeStatus::WaitingRT2Commit {
                                    Ok(trade)
                                } else {
                                    Err(anyhow::anyhow!("Invlaid request"))
                                }
                            })
                        {
                            let proc = commit_procedure::CommitProcedure::new(
                                self.storage.clone(),
                                trade,
                                self.network_client.clone(),
                                self.ethnode.clone(),
                                self.commit_proc_event_sender.clone(),
                            );
                            tokio::spawn(async move {
                                proc.verify(commit).await;
                            });

                            send_dse_response(
                                request_id,
                                self.network_client.clone(),
                                DseMessageResponse::Ack,
                            );
                        } else {
                            send_dse_response(
                                request_id,
                                self.network_client.clone(),
                                DseMessageResponse::Bad,
                            );
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

fn send_dse_response(
    request_id: RequestId,
    mut network_client: network_client::Client,
    response: network::DseMessageResponse,
) {
    tokio::spawn(async move {
        let _ = network_client
            .send_dse_message_response(request_id, response)
            .await;
    });
}
