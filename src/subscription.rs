use log::{debug, error};
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

#[derive(Debug)]
pub enum Response {
    DseRequest(network::DseMessageResponse),
    Error(anyhow::Error),
}

#[derive(Debug)]
pub struct ResponseWrapper {
    pub id: usize,
    pub response: Response,
}

pub struct Subscription {
    global_id: AtomicUsize,
    network_client: network::Client,
    sender: mpsc::Sender<ResponseWrapper>,
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
                    let res = network_client
                        .send_dse_message_request(peer_id, message)
                        .await;
                    match res {
                        Ok(dse) => {
                            debug!("(Request::DseRequest) request with id {:?} resolved", id);
                            sender
                                .send(ResponseWrapper {
                                    id,
                                    response: Response::DseRequest(dse),
                                })
                                .await;
                        }
                        Err(e) => {
                            error!(
                                "(Request::DseRequest) request with id {:?} failed with err {:?}",
                                id, e
                            );
                            sender
                                .send(ResponseWrapper {
                                    id,
                                    response: Response::Error(anyhow::anyhow!("failed!")),
                                })
                                .await;
                        }
                    }
                }
                _ => {}
            }
        });
        id
    }
}

pub fn new(network_client: network::Client) -> (mpsc::Receiver<ResponseWrapper>, Subscription) {
    let (sender, receiver) = mpsc::channel::<ResponseWrapper>(10);
    (
        receiver,
        Subscription {
            global_id: AtomicUsize::new(1),
            network_client,
            sender,
        },
    )
}
