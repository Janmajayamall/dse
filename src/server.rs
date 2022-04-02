use futures_util::{SinkExt, StreamExt, TryFutureExt};
use log::{error, info, debug};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::{select, task};
use tokio::sync::{mpsc};
use warp::ws::{WebSocket, Message};
use warp::Filter;

use super::indexer;

#[derive(Deserialize, Serialize, Debug)]
enum Commands {
    NewQuery {
        query: indexer::Query,
    },
    PlaceBid { 
        bid: indexer::Bid,
    }
}

#[derive(Debug)]
pub enum ServerEvent {
    NewWsClient{
        client_id: usize,
        client_sender: mpsc::UnboundedSender<Message>,
    },
    NewWsMessage{
        // server client which sent the message
        client_id: usize,
        message: Message
    },
    NewQuery {
        query: indexer::Query 
    },
    PlaceBid {
        bid: indexer::Bid,
    }
}

// counter for server client id
static ID_COUNTER: AtomicUsize = AtomicUsize::new(1);

pub async fn new(
    server_event_sender: mpsc::Sender<ServerEvent>,

) {
    let ws_sender = server_event_sender.clone();
    let ws_main = warp::path!("connect")
        .and(warp::ws())
        .map(move |ws: warp::ws::Ws| {
            let event_sender = ws_sender.clone(); 
            ws.on_upgrade(move |socket| ws_connection_established(socket, event_sender))
        });

    let post_query = warp::post()
        .and(warp::path("newquery"))
        // only 16kb of post data
        .and(warp::body::content_length_limit(1024 * 16))
        .and(warp::body::json())
        .and(server_event_sender.clone())
        .map(move |query: indexer::Query, event_sender | {
            event_sender.send(
                ServerEvent::NewQuery {
                    query,
                }
            );
            warp::reply()
        });
    
    let post_bid = warp::post()
        .and(warp::path("placebid"))
        // only 16kb of post data
        .and(warp::body::content_length_limit(1024 * 16))
        .and(warp::body::json())
        .and_then()
        .map(move |bid: indexer::Bid| {
            let event_sender = server_event_sender.clone();
            event_sender.send(
                ServerEvent::PlaceBid {
                    bid,
                }
            );
            warp::reply()
        });

    let main = post_bid.or(post_query).or(ws_main);

    warp::serve(main).run(([127, 0,0,1], 3000)).await;
}

async fn ws_connection_established(ws: WebSocket, event_sender: mpsc::Sender<ServerEvent>) {
    let id = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    let (mut ws_sender,mut ws_receiver) = ws.split();
    let (sender,mut receiver) = mpsc::unbounded_channel::<Message>();

    task::spawn(async move {
        loop {
            select! {
                message = receiver.recv() => { 
                    match message {
                        Some(m) => {
                            ws_sender
                            .send(m)
                            .unwrap_or_else(|e| {
                                eprintln!("Websocket send error: {}", e);
                            })
                            .await;                            
                        },
                        None => {},
                    }
                }
            }
        }
    });

    event_sender.send(ServerEvent::NewWsClient{client_id: id, client_sender: sender}).await.expect("server: NewClient message dropped!");

    loop {
        select! {
            message = ws_receiver.next() => {
                match message {
                    Some(out) => {
                        match out {
                            Ok(m) => event_sender.send(ServerEvent::NewWsMessage{client_id: id, message: m}).await.expect("server: NewClient message dropped!"),
                            Err(e) => {
                                eprintln!("server: Websocket message received error: {}", e);
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