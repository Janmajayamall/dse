use async_std::channel;
use futures_util::{SinkExt, StreamExt, TryFutureExt};
use libp2p::{identity::Keypair, PeerId};
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::{select, task};
use warp::ws::{Message, WebSocket};
use warp::{http, Filter};

use super::ethnode;
use super::indexer;
use super::network;
use super::network_client;
use super::storage;

#[derive(Debug, Clone)]
pub enum ServerEvent {
    NewWsClient {
        client_id: usize,
        client_sender: mpsc::UnboundedSender<SendWssMessage>,
    },
    NewWsMessage {
        // server client which sent the message
        client_id: usize,
        message: Message,
    },
    ReceivedMessage(ReceivedMessage),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SendWssMessage {
    /// notify client of received bid
    ReceivedBid {
        bid: storage::Bid,
        query_id: storage::QueryId,
    },

    /// notify client received query
    ReceivedQuery { query: storage::Query },

    /// notify client of bid acceptance
    ReceivedBidAcceptance { query_id: storage::QueryId },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReceivedMessage {
    /// received place bid from client
    PlaceBid { bid: storage::BidData },

    /// received query from client
    NewQuery { query: storage::QueryData },

    /// received bid acceptance from client
    AcceptBid {
        query_id: storage::QueryId,
        provider_id: PeerId,
    },

    /// received start commit from client
    StartCommit { query_id: storage::QueryId },
}

#[derive(Clone)]
pub struct Server {
    // id_counter: AtomicUsize,
    network_client: network_client::Client,
    storage: Arc<storage::Storage>,
    ethnode: ethnode::EthNode,
    keypair: Keypair,

    server_event_receiver: channel::Receiver<ServerEvent>,
    server_event_sender: channel::Sender<ServerEvent>,

    port: u16,
}

impl Server {
    pub fn new(
        network_client: network_client::Client,
        storage: Arc<storage::Storage>,
        ethnode: ethnode::EthNode,
        keypair: Keypair,
        port: u16,
    ) -> Self {
        let (server_event_sender, server_event_receiver) = channel::unbounded();
        Self {
            network_client,
            storage,
            ethnode,
            keypair,

            server_event_receiver,
            server_event_sender,

            port,
        }
    }

    pub async fn start(self) {
        let ws_main = warp::path("connect")
            .and(warp::ws())
            .and(with_server(self.clone()))
            .map(move |ws: warp::ws::Ws, server: Server| {
                ws.on_upgrade(move |socket| ws_connection_established(socket, server))
            });

        let post_action = warp::post()
            .and(warp::path("action"))
            .and(with_json_recv_message())
            .and(with_server(self.clone()))
            .and_then(handle_post);

        let get_user_queries = warp::get()
            .and(warp::path("userqueries"))
            .and(with_server(self.clone()))
            .and_then(handle_get_userqueries);

        let get_recv_queries = warp::get()
            .and(warp::path("recvqueries"))
            .and(with_server(self.clone()))
            .and_then(handle_get_recvqueries);

        let get_bids_of_query = warp::get()
            .and(warp::path!("querybids" / u32))
            .and(with_server(self.clone()))
            .and_then(handle_get_querybids);

        let main = post_action
            .or(get_user_queries)
            .or(get_recv_queries)
            .or(get_bids_of_query)
            .or(ws_main);

        warp::serve(main).run(([127, 0, 0, 1], self.port)).await;
    }
}

pub fn with_server(
    server: Server,
) -> impl Filter<Extract = (Server,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || server.clone())
}

pub fn with_json_recv_message(
) -> impl Filter<Extract = (ReceivedMessage,), Error = warp::Rejection> + Clone {
    // only 16kb of data in json body
    warp::body::content_length_limit(1024 * 16).and(warp::body::json())
}

async fn handle_post(
    recv_message: ReceivedMessage,
    mut server: Server,
) -> Result<Box<dyn warp::Reply>, std::convert::Infallible> {
    match recv_message {
        ReceivedMessage::NewQuery { query } => {
            // FIXME: Switch to chaining
            match storage::Query::from_data(
                query,
                server.keypair.public().to_peer_id(),
                server.network_client.clone(),
                server.ethnode.timelocked_wallet,
            )
            .await
            {
                Ok(query) => {
                    match server
                        .network_client
                        .publish_message(network::GossipsubMessage::NewQuery(query))
                        .await
                    {
                        Ok(_) => Ok(Box::new(http::StatusCode::OK)),
                        Err(_) => Ok(Box::new(http::StatusCode::INTERNAL_SERVER_ERROR)),
                    }
                }
                Err(_) => Ok(Box::new(http::StatusCode::INTERNAL_SERVER_ERROR)),
            }
        }
        ReceivedMessage::PlaceBid { bid } => {
            // FIXME: Switch to chaining
            match storage::Bid::from_data(
                bid,
                server.network_client.clone(),
                server.ethnode.timelocked_wallet,
            )
            .await
            {
                Ok(bid) => {
                    // Check that query for which the bid is received was received
                    // before over p2p network
                    if let Ok(query) = server
                        .storage
                        .find_query_received_by_query_id(&bid.query_id)
                    {
                        match server
                            .network_client
                            .send_dse_message_request(
                                query.requester_id,
                                network::DseMessageRequest::PlaceBid {
                                    query_id: query.id,
                                    bid: bid.clone(),
                                },
                            )
                            .await
                        {
                            Ok(network::DseMessageResponse::Ack) => {
                                {
                                    // store bid
                                    server.storage.add_bid_sent_for_query(bid, query);

                                    Ok(Box::new(http::StatusCode::OK))
                                }
                            }
                            _ => Ok(Box::new(http::StatusCode::INTERNAL_SERVER_ERROR)),
                        }
                    } else {
                        Ok(Box::new(http::StatusCode::INTERNAL_SERVER_ERROR))
                    }
                }
                Err(_) => Ok(Box::new(http::StatusCode::INTERNAL_SERVER_ERROR)),
            }
        }
        ReceivedMessage::AcceptBid {
            query_id,
            provider_id,
        } => {
            // Check that bid was received by the node (i.e. Requester)
            if let Ok((query, bid)) = server
                .storage
                .find_bid_received(&query_id, &provider_id)
                .and_then(|bid| {
                    // Find query sent for which the bid was receiveds
                    server
                        .storage
                        .find_query_sent_by_query_id(&query_id)
                        .and_then(|query| Ok((query, bid)))
                })
            {
                match server
                    .network_client
                    .send_dse_message_request(
                        bid.provider_id,
                        network::DseMessageRequest::AcceptBid { query_id },
                    )
                    .await
                {
                    Ok(network::DseMessageResponse::Ack) => {
                        // Requester does not P
                        server.storage.add_new_trade(query, bid, true);
                        Ok(Box::new(http::StatusCode::OK))
                    }
                    _ => Ok(Box::new(http::StatusCode::INTERNAL_SERVER_ERROR)),
                }
            } else {
                Ok(Box::new(http::StatusCode::INTERNAL_SERVER_ERROR))
            }
        }
        ReceivedMessage::StartCommit { query_id } => {
            // Check that Trade corresponding to query_id exists
            // since node (Provider) should have received AcceptBid from
            // Requester.
            // Make sure that status of Trade is PSendStartCommit
            if let Ok(mut trade) = server
                .storage
                .find_query_received_by_query_id(&query_id)
                .and_then(|query| {
                    server
                        .storage
                        .find_active_trade(
                            &query_id,
                            &server.keypair.public().to_peer_id(),
                            &query.requester_id,
                        )
                        .and_then(|trade| {
                            if trade.status == storage::TradeStatus::PSendStartCommit {
                                Ok(trade)
                            } else {
                                Err(anyhow::anyhow!("Invalid trade status"))
                            }
                        })
                })
            {
                match server
                    .network_client
                    .send_dse_message_request(
                        trade.query.requester_id,
                        network::DseMessageRequest::StartCommit { query_id },
                    )
                    .await
                {
                    Ok(network::DseMessageResponse::Ack) => {
                        // update trade status to WaitingRT1Commit
                        trade.update_status(storage::TradeStatus::WaitingRT1Commit);
                        server.storage.update_active_trade(trade);

                        Ok(Box::new(http::StatusCode::OK))
                    }
                    _ => Ok(Box::new(http::StatusCode::INTERNAL_SERVER_ERROR)),
                }
            } else {
                Ok(Box::new(http::StatusCode::INTERNAL_SERVER_ERROR))
            }
        }
        _ => Ok(Box::new(http::StatusCode::INTERNAL_SERVER_ERROR)),
    }
}

async fn handle_get_userqueries(
    server: Server,
) -> Result<Box<dyn warp::Reply>, std::convert::Infallible> {
    Ok(Box::new(warp::reply::json(
        &server.storage.get_all_queries_sent(),
    )))
}

async fn handle_get_recvqueries(
    server: Server,
) -> Result<Box<dyn warp::Reply>, std::convert::Infallible> {
    Ok(Box::new(warp::reply::json(
        &server.storage.get_all_queries_recv(),
    )))
}

async fn handle_get_querybids(
    query_id: u32,
    server: Server,
) -> Result<Box<dyn warp::Reply>, std::convert::Infallible> {
    Ok(Box::new(warp::reply::json(
        &server.storage.find_all_bids_recv_by_query_id(&query_id),
    )))
}

async fn ws_connection_established(ws: WebSocket, server: Server) {
    let id = server.storage.next_client_id();
    let (mut ws_sender, mut ws_receiver) = ws.split();
    let (sender, mut receiver) = mpsc::unbounded_channel::<SendWssMessage>();

    task::spawn(async move {
        loop {
            select! {
                message = receiver.recv() => {
                    match message {
                        Some(m) => {
                            ws_sender
                            .send(Message::text(serde_json::to_string(&m).unwrap()))
                            .unwrap_or_else(|e| {
                                error!("Websocket send error: {}", e);
                            })
                            .await;
                        },
                        None => {},
                    }
                }
            }
        }
    });

    server
        .server_event_sender
        .send(ServerEvent::NewWsClient {
            client_id: id,
            client_sender: sender,
        })
        .await;

    loop {
        select! {
            message = ws_receiver.next() => {
                match message {
                    Some(out) => {
                        match out {
                            Ok(m) => {server.server_event_sender.send(ServerEvent::NewWsMessage{client_id: id, message: m}).await;},
                            Err(e) => {
                                error!("server: Websocket message received error: {}", e);
                            }
                        }
                    },
                    None => {}
                }
            }
        }
    }

    println!("server: Client disconnected!");
    // TODO send Client disconnected server event
}
