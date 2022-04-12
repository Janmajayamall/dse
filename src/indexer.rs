use async_std::channel;
use libp2p::{identity::Keypair, request_response, Multiaddr, PeerId};
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tokio::{select, time};

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

    /// commitment client
    // commitment_client: commitment::Client,
    /// sent query counter
    query_counter: AtomicUsize,
    /// global database
    storage: Arc<storage::Storage>,
}

impl Indexer {
    pub fn new(
        keypair: Keypair,
        network_client: network_client::Client,
        network_event_receiver: channel::Receiver<network::NetworkEvent>,
        storage: Arc<storage::Storage>,
    ) -> Self {
        Self {
            keypair,
            server_client_senders: Default::default(),

            network_client,
            network_event_receiver,

            query_counter: AtomicUsize::new(1),
            storage,
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
                        if trade.is_sending_status() {
                            use storage::TradeStatus;
                            use network::DseMessageRequest;
                            match trade.status {
                                // TradeStatus::PSendStartCommit => {
                                //     // send start commit dse request
                                //     let mut network_client = self.network_client.clone();
                                //     let storage = self.storage.clone();
                                //     tokio::spawn(async move {
                                //         network_client.send_dse_message_request(trade.query.requester_id, DseMessageRequest::StartCommit{query_id: trade.query_id}).await;
                                //     });
                                // },
                                _ => {

                                }
                            }
                        }
                    }

                }
            }
        }
    }

    pub async fn handle_network_event(&self, event: network::NetworkEvent) {
        use network::{DseMessageRequest, GossipsubMessage, NetworkEvent};
        use storage::TradeStatus;
        match event {
            NetworkEvent::GossipsubMessageRecv(GossipsubMessage::NewQuery(query)) => {
                self.storage.add_query_received(query);
                // TODO inform client over WSS
            }
            NetworkEvent::DseMessageRequestRecv {
                peer_id,
                request_id,
                request,
            } => {
                match request {
                    DseMessageRequest::PlaceBid { query_id, bid } => {
                        // Received PlaceBid request from Requester for placing a bid
                        // for a query. Therefore, first check whether node (i.e. Requester)
                        // sent a query with given query id.
                        if let Ok(query) = self.storage.find_query_sent_by_query_id(&query_id) {
                            self.storage.add_bid_received_for_query(&query_id, bid);

                            // TODO inform the clients over WSS
                        } else {
                            // TODO send bad response
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
                                    .and_then(|query| Ok((bid, query)))
                            })
                        {
                            // is_requester = false, since node is provider
                            self.storage.add_new_trade(query, bid, false);

                            // TODO validate and store requester's wallet address.
                            // I think we should store wallet addresses of provider & requester
                            // in the trade struct.

                            // TODO inform clients over WSS
                        } else {
                            // TODO send bad response
                        }
                    }
                    DseMessageRequest::StartCommit { query_id } => {
                        // Received StartCommit from Provider for AcceptedBid on a published
                        // Query by the Node (i.e. Requester). Thus, Trade object should exist
                        // for the given query id with provider id as peer id.
                        if let Ok(trade) = self
                            .storage
                            .find_active_trade(
                                &query_id,
                                &peer_id,
                                &self.keypair.public().to_peer_id(),
                            )
                            .and_then(|trade| {
                                if trade.status == TradeStatus::WaitingStartCommit {
                                    Ok(trade)
                                } else {
                                    Err(anyhow::anyhow!("Invlaid request"))
                                }
                            })
                        {
                            // TODO prepare for t2 commits
                            todo!();
                        } else {
                            // TODO send bad response
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
                                &peer_id,
                            )
                            .and_then(|trade| {
                                if trade.status == TradeStatus::WaitingRT1Commit {
                                    Ok(trade)
                                } else {
                                    Err(anyhow::anyhow!("Invlaid request"))
                                }
                            })
                        {
                            // TODO check that commit indexes valid and then update the state to PSendT1Commit
                            todo!();
                        } else {
                            // TODO send bad resoponse or can even send expected value
                        }
                    }
                    DseMessageRequest::T1ProviderCommit { query_id, commit } => {
                        // Received T1 Commit from provider. Therefore, TradeStatus of the node
                        // (i.e. Requester) should be WaitingPT1Commit
                        if let Ok(trade) = self
                            .storage
                            .find_active_trade(
                                &query_id,
                                &peer_id,
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
                            // TODO check that commit indexes valid and then update the state to RSendT2Commit
                            todo!();
                        } else {
                            // TODO send bad resoponse or can even send expected value
                        }
                    }
                    DseMessageRequest::T2RequesterCommit { query_id, commit } => {
                        // Received T2 Commit from requester.
                        if let Ok(trade) = self
                            .storage
                            .find_active_trade(
                                &query_id,
                                &peer_id,
                                &self.keypair.public().to_peer_id(),
                            )
                            .and_then(|trade| {
                                if trade.status == TradeStatus::WaitingRT2Commit {
                                    Ok(trade)
                                } else {
                                    Err(anyhow::anyhow!("Invlaid request"))
                                }
                            })
                        {
                            // TODO check that commit indexes valid and then update the state to RSendT2Commit
                            todo!();
                        } else {
                            // TODO send bad resoponse or can even send expected value
                        }
                    }

                    _ => {}
                }
            }
            _ => {}
        }
    }
}
