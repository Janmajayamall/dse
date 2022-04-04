use futures_util::{SinkExt, StreamExt, TryFutureExt};
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::mpsc;
use tokio::{select, task};
use warp::ws::{Message, WebSocket};
use warp::{http, Filter};

use super::indexer;

#[derive(Deserialize, Serialize, Debug)]
enum Commands {
    NewQuery { query: indexer::Query },
    PlaceBid { bid: indexer::Bid },
}

#[derive(Debug)]
pub enum ServerEvent {
    NewWsClient {
        client_id: usize,
        client_sender: mpsc::UnboundedSender<Message>,
    },
    NewWsMessage {
        // server client which sent the message
        client_id: usize,
        message: Message,
    },
    NewQuery {
        query: indexer::Query,
    },
    PlaceBid {
        bid: indexer::Bid,
    },
}

// counter for server client id
static ID_COUNTER: AtomicUsize = AtomicUsize::new(1);

pub fn with_sender<T: Send + Sync>(
    sender: mpsc::Sender<T>,
) -> impl Filter<Extract = (mpsc::Sender<T>,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || sender.clone())
}

pub async fn new(server_event_sender: mpsc::Sender<ServerEvent>) {
    let ws_main = warp::path("connect")
        .and(warp::ws())
        .and(with_sender(server_event_sender.clone()))
        .map(
            move |ws: warp::ws::Ws, event_sender: mpsc::Sender<ServerEvent>| {
                ws.on_upgrade(move |socket| ws_connection_established(socket, event_sender))
            },
        );

    let post_query = warp::post()
        .and(warp::path("newquery"))
        // only 16kb of post data
        .and(warp::body::content_length_limit(1024 * 16))
        .and(warp::body::json())
        .and(with_sender(server_event_sender.clone()))
        .and_then(handle_newquery);

    let post_bid = warp::post()
        .and(warp::path("placebid"))
        // only 16kb of post data
        .and(warp::body::content_length_limit(1024 * 16))
        .and(warp::body::json())
        .and(with_sender(server_event_sender.clone()))
        .and_then(handle_placebid);

    let main = post_bid.or(post_query).or(ws_main);

    warp::serve(main).run(([127, 0, 0, 1], 3000)).await;
}

async fn handle_newquery(
    query: indexer::Query,
    event_sender: mpsc::Sender<ServerEvent>,
) -> Result<impl warp::Reply, std::convert::Infallible> {
    let _ = event_sender.send(ServerEvent::NewQuery { query }).await;
    Ok(http::StatusCode::OK)
}

async fn handle_placebid(
    bid: indexer::Bid,
    event_sender: mpsc::Sender<ServerEvent>,
) -> Result<impl warp::Reply, std::convert::Infallible> {
    let _ = event_sender.send(ServerEvent::PlaceBid { bid }).await;
    Ok(http::StatusCode::OK)
}

async fn ws_connection_established(ws: WebSocket, event_sender: mpsc::Sender<ServerEvent>) {
    let id = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    let (mut ws_sender, mut ws_receiver) = ws.split();
    let (sender, mut receiver) = mpsc::unbounded_channel::<Message>();

    task::spawn(async move {
        loop {
            select! {
                message = receiver.recv() => {
                    match message {
                        Some(m) => {
                            ws_sender
                            .send(m)
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

    event_sender
        .send(ServerEvent::NewWsClient {
            client_id: id,
            client_sender: sender,
        })
        .await
        .expect("server: NewClient message dropped!");

    loop {
        select! {
            message = ws_receiver.next() => {
                match message {
                    Some(out) => {
                        match out {
                            Ok(m) => event_sender.send(ServerEvent::NewWsMessage{client_id: id, message: m}).await.expect("server: NewClient message dropped!"),
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
