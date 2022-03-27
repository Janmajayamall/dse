
use futures_util::{SinkExt, StreamExt, TryFutureExt};
use tokio::{select, task};
use tokio::sync::{mpsc};
use warp::ws::{WebSocket, Message};
use warp::Filter;

#[derive(Debug)]
enum ServerEvent {
    NewWsClient(mpsc::UnboundedSender<Message>),
    NewWsMessage(Message),
}

pub fn new(
    server_event_sender: mpsc::Sender<ServerEvent>,

) {
    let main = warp::path!("connect")
        .and(warp::ws())
        .map(|ws: warp::ws::Ws| {
            let event_sender = server_event_sender.clone(); 
            ws.on_upgrade(move |socket| ws_connection_established(socket, event_sender))
        });

     
}

async fn ws_connection_established(ws: WebSocket, event_sender: mpsc::Sender<ServerEvent>) {
    let (mut ws_sender,mut ws_receiver) = ws.split();

    let (mut sender,mut receiver) = mpsc::unbounded_channel::<Message>();

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

    event_sender.send(ServerEvent::NewWsClient(sender)).await.expect("server: NewClient message dropped!");

    loop {
        select! {
            message = ws_receiver.next() => {
                // message received over websockets
            }
        }
    }
}