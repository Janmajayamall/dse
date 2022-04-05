use futures_util::{SinkExt, StreamExt, TryFutureExt};
use libp2p::PeerId;
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::mpsc;
use tokio::{select, task};
use warp::ws::{Message, WebSocket};
use warp::{http, Filter};

use super::database;
use super::indexer;

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
        bid: indexer::BidReceivedWithStatus,
        query_id: indexer::QueryId,
    },

    /// notify client received query
    ReceivedQuery { query: indexer::QueryReceived },

    /// notify client of bid acceptance
    ReceivedBidAcceptance { query_id: indexer::QueryId },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReceivedMessage {
    /// received place bid from client
    PlaceBid { bid: indexer::Bid },

    /// received query from client
    NewQuery { query: indexer::Query },

    /// received bid acceptance from client
    AcceptBid {
        query_id: indexer::QueryId,
        bidder_id: PeerId,
    },

    /// received start commit from client
    StartCommit { query_id: indexer::QueryId },
}

// counter for server client id
static ID_COUNTER: AtomicUsize = AtomicUsize::new(1);

pub fn with_sender<T: Send + Sync>(
    sender: mpsc::Sender<T>,
) -> impl Filter<Extract = (mpsc::Sender<T>,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || sender.clone())
}

pub fn with_indexer_client(
    client: indexer::Client,
) -> impl Filter<Extract = (indexer::Client,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || client.clone())
}

pub fn with_database(
    db: database::Database,
) -> impl Filter<Extract = (database::Database,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || db.clone())
}

pub fn with_json_recv_message(
) -> impl Filter<Extract = (ReceivedMessage,), Error = warp::Rejection> + Clone {
    // only 16kb of data in json body
    warp::body::content_length_limit(1024 * 16).and(warp::body::json())
}

pub async fn new(indexer_client: indexer::Client, db: database::Database, port: u16) {
    let ws_main = warp::path("connect")
        .and(warp::ws())
        .and(with_indexer_client(indexer_client.clone()))
        .map(move |ws: warp::ws::Ws, indexer_client: indexer::Client| {
            ws.on_upgrade(move |socket| ws_connection_established(socket, indexer_client))
        });

    let post_action = warp::post()
        .and(warp::path("action"))
        .and(with_json_recv_message())
        .and(with_indexer_client(indexer_client.clone()))
        .and_then(handle_post);

    let get_user_queries = warp::get()
        .and(warp::path("userqueries"))
        .and(with_database(db.clone()))
        .and_then(handle_get_userqueries);

    let get_recv_queries = warp::get()
        .and(warp::path("recvqueries"))
        .and(with_database(db.clone()))
        .and_then(handle_get_recvqueries);

    let get_bids_of_query = warp::get()
        .and(warp::path!("querybids" / u32))
        .and(with_database(db.clone()))
        .and_then(handle_get_querybids);

    let main = post_action
        .or(get_user_queries)
        .or(get_recv_queries)
        .or(get_bids_of_query)
        .or(ws_main);

    warp::serve(main).run(([127, 0, 0, 1], port)).await;
}

async fn handle_post(
    recv_message: ReceivedMessage,
    mut indexer_client: indexer::Client,
) -> Result<Box<dyn warp::Reply>, std::convert::Infallible> {
    match indexer_client
        .handle_server_event(ServerEvent::ReceivedMessage(recv_message))
        .await
    {
        Ok(_) => Ok(Box::new(http::StatusCode::OK)),
        Err(e) => {
            error!("(post: action) errored with {:?}", e);
            Ok(Box::new(http::StatusCode::INTERNAL_SERVER_ERROR))
        }
    }

    // match recv_message {

    //     ReceivedWssMessage::NewQuery { query } => {
    //         let _ = event_sender.send(ServerEvent::NewQuery { query }).await;
    //         Ok(http::StatusCode::OK)
    //     }
    //     ReceivedWssMessage::PlaceBid { bid } => {
    //         let _ = event_sender.send(ServerEvent:: { bid }).await;
    //         Ok(http::StatusCode::OK)
    //     }
    //     ReceivedWssMessage::AcceptBid {
    //         query_id,
    //         bidder_id,
    //     } => {

    //     }
    //     _ =>
    // }
}

async fn handle_get_userqueries(
    db: database::Database,
) -> Result<Box<dyn warp::Reply>, std::convert::Infallible> {
    match db.find_user_queries() {
        Ok(queries) => Ok(Box::new(warp::reply::json(&queries))),
        Err(e) => {
            error!("(get: userqueries) errored with {:?}", e);
            Ok(Box::new(http::StatusCode::INTERNAL_SERVER_ERROR))
        }
    }
}

async fn handle_get_recvqueries(
    db: database::Database,
) -> Result<Box<dyn warp::Reply>, std::convert::Infallible> {
    match db.find_recv_queries() {
        Ok(queries) => Ok(Box::new(warp::reply::json(&queries))),
        Err(e) => {
            error!("(get: userqueries) errored with {:?}", e);
            Ok(Box::new(http::StatusCode::INTERNAL_SERVER_ERROR))
        }
    }
}

async fn handle_get_querybids(
    query_id: u32,
    db: database::Database,
) -> Result<Box<dyn warp::Reply>, std::convert::Infallible> {
    match db.find_query_bids_with_status(&query_id) {
        Ok(bids) => Ok(Box::new(warp::reply::json(&bids))),
        Err(e) => {
            error!("(get: querybids) errored with {:?}", e);
            Ok(Box::new(http::StatusCode::INTERNAL_SERVER_ERROR))
        }
    }
}

async fn ws_connection_established(ws: WebSocket, mut indexer_client: indexer::Client) {
    let id = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
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

    indexer_client
        .handle_server_event(ServerEvent::NewWsClient {
            client_id: id,
            client_sender: sender,
        })
        .await
        .expect("server: new wss client event command to indexer dropped!");

    loop {
        select! {
            message = ws_receiver.next() => {
                match message {
                    Some(out) => {
                        match out {
                            Ok(m) => indexer_client.handle_server_event(ServerEvent::NewWsMessage{client_id: id, message: m}).await.expect("server: new wss message dropped!"),
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
