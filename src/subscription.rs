use log::debug;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{mpsc, oneshot};

use super::network;

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    DseRequest {
        peer_id: libp2p::PeerId,
        message: network::DseMessageRequest,
    },
    DhtPut,
    DhtGet,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub dseMessageResponse: network::DseMessageResponse,
    pub id: usize,
}

pub struct Subscription {
    global_id: AtomicUsize,
    network_client: network::Client,
    sender: mpsc::Sender<Response>,
}

impl Subscription {
    pub fn start(&mut self, request: Request) -> usize {
        let id = self.global_id.fetch_add(1, Ordering::Relaxed);
        let mut network_client = self.network_client.clone();
        let sender = self.sender.clone();
        tokio::spawn(async move {
            debug!(
                "Starting subscription with id {:?} for network request {:?}",
                id, request
            );
            match request {
                Request::DseRequest { peer_id, message } => {
                    match network_client
                        .send_dse_message_request(peer_id, message)
                        .await
                    {
                        Ok(res) => {
                            debug!(
                                "(Request::DseRequest) request with id {:?} resolved with response {:?}",
                                id, res
                            );
                            sender
                                .send(Response {
                                    dseMessageResponse: res,
                                    id,
                                })
                                .await;
                        }
                        Err(_) => {}
                    };
                }
                _ => {}
            }
        });
        id
    }
}

pub fn new(network_client: network::Client) -> (mpsc::Receiver<Response>, Subscription) {
    let (sender, receiver) = mpsc::channel::<Response>(10);
    (
        receiver,
        Subscription {
            global_id: AtomicUsize::new(1),
            network_client,
            sender,
        },
    )
}
