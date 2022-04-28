use super::commit_procedure;
use super::ethnode;
use super::network_client;
use super::server;
use super::storage;
use crate::network::{
    CommitRequest, CommitResponse, ExchangeRequest, ExchangeResponse, GossipsubMessage,
    NetworkEvent,
};
use libp2p::{identity::Keypair, request_response::RequestId, PeerId};
use log::debug;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tokio::{select, time};
/// Main interface thru which user interacts.
/// That means sends and receives querues & bids.
pub struct Indexer {
    keypair: Keypair,
    /// Sends events to server clients over websocket
    server_client_senders: HashMap<usize, mpsc::UnboundedSender<server::SendWssMessage>>,

    /// network client
    network_client: network_client::Client,
    network_event_receiver: broadcast::Receiver<NetworkEvent>,

    /// global database
    storage: Arc<storage::Storage>,
    ethnode: ethnode::EthNode,

    // receiver of commit procedure events
    commit_proc_event_receiver: mpsc::Receiver<commit_procedure::CommitProcedureEvent>,
    // sender for commit procedure events
    commit_proc_event_sender: mpsc::Sender<commit_procedure::CommitProcedureEvent>,

    // query ids of trades in send status currently
    // with active send() CommitProcedure
    active_send_commit_proc: HashSet<u32>,
}

impl Indexer {
    pub fn new(
        keypair: Keypair,
        network_client: network_client::Client,
        network_event_receiver: broadcast::Receiver<NetworkEvent>,
        storage: Arc<storage::Storage>,
        ethnode: ethnode::EthNode,
    ) -> Self {
        let (commit_proc_event_sender, commit_proc_event_receiver) =
            mpsc::channel::<commit_procedure::CommitProcedureEvent>(20);

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
                        if trade.is_sending_status() && !self.active_send_commit_proc.contains(&trade.id) {
                            self.active_send_commit_proc.insert(trade.id);

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

                        // TODO: Send WS message to clients if trade status
                        // is PFulfill Service
                    }

                },
                Some(event) = self.commit_proc_event_receiver.recv() => {
                    use commit_procedure::CommitProcedureEvent;
                    match event {
                        CommitProcedureEvent::SendSuccess{ trade_id } => {
                            self.active_send_commit_proc.remove(&trade_id);
                        },
                        CommitProcedureEvent::SendFailed{ trade_id } => {
                            self.active_send_commit_proc.remove(&trade_id);
                        }
                    };
                }
            }
        }
    }

    pub async fn handle_network_event(&self, event: NetworkEvent) {
        use storage::TradeStatus;
        match event {
            NetworkEvent::GossipsubMessageRecv(GossipsubMessage::NewQuery(query)) => {
                debug!(
                    "Received new query with id {} from requester_id {}",
                    query.id, query.requester_id
                );

                // add requester address to request response
                // _ = self
                //     .network_client
                //     .add_request_response_peer(query.requester_id, query.requester_addr.clone())
                //     .await;

                self.storage.add_query_received(query);
                // TODO inform client over WSS
            }
            NetworkEvent::GossipsubMessageRecv(GossipsubMessage::Commit { commit, trade_id }) => {
                debug!(
                    "Received new commit for wallet address {} for trade id {}",
                    commit.wallet_address, trade_id
                );

                if self
                    .storage
                    .get_wallets_of_interest()
                    .exists(&commit.wallet_address)
                {
                    // TODO: Check that commit is valid

                    // Add commit to commit history
                    self.storage.add_commits_to_commit_history(
                        &commit.wallet_address,
                        vec![commit.clone()],
                    );
                }
            }
            NetworkEvent::GossipsubMessageRecv(GossipsubMessage::InvalidatingSignature {
                provider_wallet,
                requester_wallet,
                invalidating_signature,
                trade_id,
            }) => {
                debug!("Received invalidating signature for trade id {}", trade_id);

                if self
                    .storage
                    .get_wallets_of_interest()
                    .exists(&provider_wallet)
                {
                    self.storage
                        .update_commit_history_with_invalidating_signature(
                            &trade_id,
                            &provider_wallet,
                            &invalidating_signature,
                        )
                }

                if self
                    .storage
                    .get_wallets_of_interest()
                    .exists(&requester_wallet)
                {
                    self.storage
                        .update_commit_history_with_invalidating_signature(
                            &trade_id,
                            &requester_wallet,
                            &invalidating_signature,
                        )
                }
            }
            NetworkEvent::ExchangeRequest {
                sender_peer_id,
                request_id,
                request,
            } => {
                match request {
                    // DseMessageRequest::PlaceBid { query_id, bid } => {
                    //     // // Received PlaceBid request from Requester for placing a bid
                    //     // // for a query. Therefore, first check whether node (i.e. Requester)
                    //     // // sent a query with given query id.
                    //     // if self
                    //     //     .storage
                    //     //     .find_query_sent_by_query_id(&query_id)
                    //     //     .and_then(|q| {
                    //     //         // provider_id should match sender_peer_id
                    //     //         // from whom request was received
                    //     //         if bid.provider_id == sender_peer_id {
                    //     //             Ok(q)
                    //     //         } else {
                    //     //             Err(anyhow::anyhow!("Peer id mismatch"))
                    //     //         }
                    //     //     })
                    //     //     .is_ok()
                    //     // {
                    //     //     debug!(
                    //     //         "PlaceBid request received for query_id {} from provider_id {}",
                    //     //         query_id, bid.provider_id
                    //     //     );

                    //     //     // add provider address to request response
                    //     //     // _ = self
                    //     //     //     .network_client
                    //     //     //     .add_request_response_peer(
                    //     //     //         bid.provider_id,
                    //     //     //         bid.provider_addr.clone(),
                    //     //     //     )
                    //     //     //     .await;

                    //     //     self.storage.add_bid_received_for_query(&query_id, bid);
                    //     //     send_dse_response(
                    //     //         request_id,
                    //     //         self.network_client.clone(),
                    //     //         DseMessageResponse::Ack,
                    //     //     );

                    //     //     // TODO inform the clients over WSS
                    //     // } else {
                    //     //     send_dse_response(
                    //     //         request_id,
                    //     //         self.network_client.clone(),
                    //     //         DseMessageResponse::Bad,
                    //     //     );
                    //     // }
                    // }
                    // DseMessageRequest::AcceptBid { query_id } => {
                    //     // // Received AcceptBid from Requester for bid placed by Node
                    //     // // (i.e. Provider) on their query with given query id.
                    //     // // Therefore, first check that bid was placed & query was received by
                    //     // // the Node.
                    //     // if let Ok((bid, query)) = self
                    //     //     .storage
                    //     //     .find_bid_sent_by_query_id(&query_id)
                    //     //     .and_then(|bid| {
                    //     //         self.storage
                    //     //             .find_query_received_by_query_id(&query_id)
                    //     //             .and_then(|query| {
                    //     //                 // Check that sender is the requester of query
                    //     //                 if query.requester_id == sender_peer_id {
                    //     //                     Ok((bid, query))
                    //     //                 } else {
                    //     //                     Err(anyhow::anyhow!("Peer id mismatch"))
                    //     //                 }
                    //     //             })
                    //     //     })
                    //     // {
                    //     //     debug!(
                    //     //         "AcceptBid request received for query_id {} from requester_id {}",
                    //     //         query_id, sender_peer_id
                    //     //     );

                    //     //     // is_requester = false, since node is provider
                    //     //     // FIXME:
                    //     //     // self.storage.add_new_trade(query, bid, false);

                    //     //     send_dse_response(
                    //     //         request_id,
                    //     //         self.network_client.clone(),
                    //     //         DseMessageResponse::Ack,
                    //     //     );

                    //     //     // TODO inform clients over WSS
                    //     // } else {
                    //     //     send_dse_response(
                    //     //         request_id,
                    //     //         self.network_client.clone(),
                    //     //         DseMessageResponse::Bad,
                    //     //     );
                    //     //     // TODO send bad response
                    //     // }
                    // }
                    ExchangeRequest::IWant { i_want } => {
                        _ = self.storage.add_i_want(&i_want);

                        // TODO send over WSS

                        send_exchange_response(
                            sender_peer_id,
                            request_id,
                            self.network_client.clone(),
                            ExchangeResponse::Ack,
                        );
                    }
                    ExchangeRequest::StartCommit { mut trade } => {
                        // TODO: validate received trade from provider

                        // Update status to RSendT1Commit
                        trade.update_status(storage::TradeStatus::RSendT1Commit);
                        _ = self.storage.add_active_trade(&trade);

                        // TODO send over WSS
                        send_exchange_response(
                            sender_peer_id,
                            request_id,
                            self.network_client.clone(),
                            ExchangeResponse::Ack,
                        );

                        // // Received StartCommit from Provider for AcceptedBid on a published
                        // // Query by the Node (i.e. Requester). Thus, Trade object should exist
                        // // for the given query id with provider id as sender id.
                        // if let Ok(mut trade) = self
                        //     .storage
                        //     .find_active_trade(
                        //         &query_id,
                        //         // request sender should be the provider in the trade
                        //         &sender_peer_id,
                        //         &self.keypair.public().to_peer_id(),
                        //     )
                        //     .and_then(|trade| {
                        //         if trade.status == TradeStatus::WaitingStartCommit {
                        //             Ok(trade)
                        //         } else {
                        //             Err(anyhow::anyhow!("Invalid request"))
                        //         }
                        //     })
                        // {
                        //     debug!(
                        //         "StartCommit request received for query_id {} from provider_id {}",
                        //         query_id, sender_peer_id
                        //     );

                        //     // update waiting status to RSendT1Commit
                        //     trade.update_status(TradeStatus::RSendT1Commit);
                        //     _ = self.storage.update_active_trade(trade);

                        //     send_dse_response(
                        //         request_id,
                        //         self.network_client.clone(),
                        //         DseMessageResponse::Ack,
                        //     );
                        // } else {
                        //     send_dse_response(
                        //         request_id,
                        //         self.network_client.clone(),
                        //         DseMessageResponse::Bad,
                        //     );
                        // }
                    }
                    ExchangeRequest::T1RequesterCommit { trade_id, commit } => {
                        if let Ok(trade) =
                            self.storage.find_active_trade(&trade_id).and_then(|trade| {
                                if trade.status == TradeStatus::WaitingRT1Commit
                                    && trade.counter_party_details().0 == sender_peer_id
                                {
                                    Ok(trade)
                                } else {
                                    Err(anyhow::anyhow!("Invalid request"))
                                }
                            })
                        {
                            debug!(
                                "T1RequesterCommit request received for trade id {} from requester id {}",
                                trade_id, sender_peer_id
                            );

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

                            send_exchange_response(
                                sender_peer_id,
                                request_id,
                                self.network_client.clone(),
                                ExchangeResponse::Ack,
                            );
                        } else {
                            send_exchange_response(
                                sender_peer_id,
                                request_id,
                                self.network_client.clone(),
                                ExchangeResponse::Bad,
                            );
                        }
                    }
                    ExchangeRequest::T1ProviderCommit { trade_id, commit } => {
                        // Received T1 Commit from provider. Therefore, TradeStatus of the node
                        // (i.e. Requester) should be WaitingPT1Commit
                        if let Ok(trade) =
                            self.storage.find_active_trade(&trade_id).and_then(|trade| {
                                if trade.status == TradeStatus::WaitingPT1Commit
                                    && trade.counter_party_details().0 == sender_peer_id
                                {
                                    Ok(trade)
                                } else {
                                    Err(anyhow::anyhow!("Invlaid request"))
                                }
                            })
                        {
                            debug!(
                                "T1ProviderCommit request received for trade id {} from provider id {}",
                                trade_id, sender_peer_id
                            );

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

                            send_exchange_response(
                                sender_peer_id,
                                request_id,
                                self.network_client.clone(),
                                ExchangeResponse::Ack,
                            );
                        } else {
                            send_exchange_response(
                                sender_peer_id,
                                request_id,
                                self.network_client.clone(),
                                ExchangeResponse::Bad,
                            );
                        }
                    }
                    ExchangeRequest::T2RequesterCommit { trade_id, commit } => {
                        // Received T2 Commit from requester.
                        if let Ok(trade) =
                            self.storage.find_active_trade(&trade_id).and_then(|trade| {
                                if trade.status == TradeStatus::WaitingRT2Commit
                                    && trade.counter_party_details().0 == sender_peer_id
                                {
                                    Ok(trade)
                                } else {
                                    Err(anyhow::anyhow!("Invlaid request"))
                                }
                            })
                        {
                            debug!(
                                "T2RequesterCommit request received for trade id {} from requester id {}",
                                trade_id, sender_peer_id
                            );

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

                            send_exchange_response(
                                sender_peer_id,
                                request_id,
                                self.network_client.clone(),
                                ExchangeResponse::Ack,
                            );
                        } else {
                            send_exchange_response(
                                sender_peer_id,
                                request_id,
                                self.network_client.clone(),
                                ExchangeResponse::Bad,
                            );
                        }
                    }
                    _ => {}
                }
            }
            NetworkEvent::CommitRequest {
                sender_peer_id,
                request_id,
                request: CommitRequest::WantHistory { wallet_address },
            } => {
                if let Some(commit_history) = self.storage.find_commit_history(&wallet_address) {
                    // send Ack
                    send_commit_response(
                        sender_peer_id,
                        request_id,
                        self.network_client.clone(),
                        CommitResponse::Ack,
                    );

                    // FIXME: There are lot of things wrong here -
                    // 1. Divide all commits into groups and send them over
                    // few requests, so that each request data size is small.
                    // 2. Don't handle this here. Spwan a new thread with a struct
                    // to handle sending CommitHistory to a peer.
                    let nc = self.network_client.clone();
                    tokio::spawn(async move {
                        let _ = nc
                            .send_commit_request(
                                sender_peer_id,
                                CommitRequest::Update {
                                    wallet_address,
                                    commits: commit_history.commits,
                                    last_batch: true,
                                },
                            )
                            .await;
                    });
                } else {
                    // Commit history does not exists
                    // send back bad response
                    send_commit_response(
                        sender_peer_id,
                        request_id,
                        self.network_client.clone(),
                        CommitResponse::Bad,
                    );
                }
            }
            _ => {}
        }
    }
}

fn send_exchange_response(
    peer_id: PeerId,
    request_id: RequestId,
    mut network_client: network_client::Client,
    response: ExchangeResponse,
) {
    tokio::spawn(async move {
        let _ = network_client
            .send_exchange_response(peer_id, request_id, response)
            .await;
    });
}

fn send_commit_response(
    peer_id: PeerId,
    request_id: RequestId,
    mut network_client: network_client::Client,
    response: CommitResponse,
) {
    tokio::spawn(async move {
        let _ = network_client
            .send_commit_response(peer_id, request_id, response)
            .await;
    });
}
