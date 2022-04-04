use libp2p::{Multiaddr, PeerId};
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::select;
use tokio::sync::{mpsc, oneshot};

use super::commitment;
use super::network;
use super::server;

pub type QueryId = u32;

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct Query {
    query: String,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct Bid {
    pub query_id: QueryId,
    // peer id of query requester
    // to whom this bid is placed
    pub requester_id: PeerId,
    // charge for query in cents
    pub charge: ethers::types::U256,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BidReceived {
    pub bidder_id: PeerId,
    pub bidder_addr: Multiaddr,
    pub query_id: QueryId,
    pub bid: Bid,
}

impl BidReceived {
    pub fn from(bid: Bid, bidder_id: PeerId, bidder_addr: Multiaddr) -> Self {
        Self {
            bid: bid.clone(),
            query_id: bid.query_id,
            bidder_id,
            bidder_addr,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct QueryReceived {
    pub id: QueryId,
    pub requester_id: PeerId,
    pub requester_addr: Multiaddr,
    pub query: Query,
}

impl QueryReceived {
    // pub fn from(query: Query, )
}

#[derive(Debug)]
pub enum Command {
    ReceivedBid {
        bid_recv: BidReceived,
        sender: oneshot::Sender<Result<(), anyhow::Error>>,
    },
    ReceivedQuery {
        query_recv: QueryReceived,
        sender: oneshot::Sender<Result<(), anyhow::Error>>,
    },
    ReceivedBidAcceptance {
        query_id: QueryId,
        peer_id: PeerId,
        sender: oneshot::Sender<Result<(), anyhow::Error>>,
    },
    ReceivedStartCommit {
        query_id: QueryId,
        peer_id: PeerId,
        sender: oneshot::Sender<Result<(), anyhow::Error>>,
    },
}

#[derive(Debug)]
pub enum IndexerEvent {
    SendDseMessageRequest {
        request: network::DseMessageRequest,
        send_to: PeerId,
    },
    NewQuery {
        query: Query,
    },
    PlaceBid {
        bid: Bid,
    },
    RequestNodeMultiAddr {
        sender: oneshot::Sender<Result<Multiaddr, anyhow::Error>>, // TODO change it to channel
    },
}

#[derive(Clone)]
pub struct Client {
    pub command_sender: mpsc::Sender<Command>,
}

// Client receives commands and forwards them
impl Client {
    pub async fn handle_received_bid(
        &mut self,
        bid_recv: BidReceived,
    ) -> Result<(), anyhow::Error> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::ReceivedBid { bid_recv, sender })
            .await
            .expect("Command message dropped");
        receiver.await.expect("Command response dropped")
    }

    pub async fn handle_received_query(
        &mut self,
        query_recv: QueryReceived,
    ) -> Result<(), anyhow::Error> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::ReceivedQuery { query_recv, sender })
            .await
            .expect("Command message dropped");
        receiver.await.expect("Command response dropped")
    }

    pub async fn handle_received_bid_acceptance(
        &mut self,
        query_id: QueryId,
        peer_id: PeerId,
    ) -> Result<(), anyhow::Error> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::ReceivedBidAcceptance {
                query_id,
                peer_id,
                sender,
            })
            .await
            .expect("Command message dropped");
        receiver.await.expect("Command response dropped")
    }

    pub async fn handle_received_start_commit(
        &mut self,
        query_id: QueryId,
        peer_id: PeerId,
    ) -> Result<(), anyhow::Error> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::ReceivedStartCommit {
                query_id,
                peer_id,
                sender,
            })
            .await
            .expect("Command message dropped");
        receiver.await.expect("Command response dropped")
    }
}

/// Main interface thru which user interacts.
/// That means sends and receives querues & bids.
pub struct Indexer {
    /// Receives indexer commands
    command_receiver: mpsc::Receiver<Command>,
    /// FIX - I think this is useless
    event_sender: mpsc::Sender<IndexerEvent>,
    /// Receives events from the server
    server_event_receiver: mpsc::Receiver<server::ServerEvent>,
    /// Sends events to server
    server_client_senders: HashMap<usize, mpsc::UnboundedSender<warp::ws::Message>>,
    /// network client
    network_client: network::Client,
    /// commitment client
    commitment_client: commitment::Client,
    /// sent query counter
    query_counter: AtomicUsize,
}

impl Indexer {
    pub fn new(
        command_receiver: mpsc::Receiver<Command>,
        event_sender: mpsc::Sender<IndexerEvent>,
        server_event_receiver: mpsc::Receiver<server::ServerEvent>,
        network_client: network::Client,
        commitment_client: commitment::Client,
    ) -> Self {
        Self {
            command_receiver,
            event_sender,
            server_event_receiver,
            server_client_senders: Default::default(),
            network_client,
            commitment_client,
            query_counter: AtomicUsize::new(1),
        }
    }

    pub async fn run(mut self) {
        loop {
            select! {
                command = self.command_receiver.recv() => {
                    match command {
                        Some(c) => self.command_handler(c).await,
                        None => {}
                    }
                },
                event = self.server_event_receiver.recv() => {
                    match event {
                        Some(e) => self.server_event_handler(e).await,
                        None => {}
                    }
                }
            }
        }
    }

    // TODO convert indexer command messages to websocket messages
    pub async fn command_handler(&mut self, command: Command) {
        match command {
            Command::ReceivedBid { bid_recv, sender } => {
                self.network_client
                    .add_request_response_peer(
                        bid_recv.bidder_id.clone(),
                        bid_recv.bidder_addr.clone(),
                    )
                    .await;
                // TODO something with the query
                sender.send(Ok(()));
                debug!(
                    "indexer: received bid for query_id {:?} from bidder {:?} with addr {:?} ",
                    bid_recv.bid.query_id, bid_recv.bidder_id, bid_recv.bidder_addr
                );
            }
            Command::ReceivedQuery { query_recv, sender } => {
                self.network_client
                    .add_request_response_peer(
                        query_recv.requester_id.clone(),
                        query_recv.requester_addr.clone(),
                    )
                    .await;
                // TODO something with query
                sender.send(Ok(()));
                debug!("indexer: received query with query_id {:?} from requester {:?} with addr {:?} ", query_recv.id, query_recv.requester_id, query_recv.requester_addr);
            }
            Command::ReceivedBidAcceptance {
                query_id,
                peer_id,
                sender,
            } => {
                sender.send(Ok(()));
                debug!(
                    "indexer: received bid acceptance for query_id {:?} from requester {:?} ",
                    query_id, peer_id
                );
            }
            Command::ReceivedStartCommit {
                query_id,
                peer_id,
                sender,
            } => {
                // notify wallet for commitment
                sender.send(Ok(()));
                debug!(
                    "indexer: received start commit for query_id {:?} from peer {:?} ",
                    query_id, peer_id
                );
            }
            _ => {}
        }
    }

    pub async fn server_event_handler(&mut self, event: server::ServerEvent) {
        use server::ServerEvent;
        match event {
            ServerEvent::NewWsClient {
                client_id,
                client_sender,
            } => {
                self.server_client_senders.insert(client_id, client_sender);
            }
            ServerEvent::NewWsMessage { client_id, message } => {
                // TODO handle message
                // request node id using indexer event of RequestNodeMultiAddr
            }
            ServerEvent::NewQuery { query } => match self.network_client.network_details().await {
                Ok((peer_id, address)) => {
                    let id = self.query_counter.fetch_add(1, Ordering::Relaxed);
                    let query_recv = QueryReceived {
                        id: id.try_into().expect("indexer: Query limit reached"),
                        requester_id: peer_id,
                        requester_addr: address,
                        query,
                    };
                    self.network_client
                        .publish_message(network::GossipsubMessage::NewQuery(query_recv))
                        .await;
                }
                Err(_) => {}
            },
            ServerEvent::PlaceBid { bid } => match self.network_client.network_details().await {
                Ok((peer_id, address)) => {
                    let bid_recv = BidReceived {
                        bidder_id: peer_id,
                        bidder_addr: address,
                        query_id: bid.query_id,
                        bid,
                    };
                    let _ = self
                        .network_client
                        .send_dse_message_request(
                            peer_id,
                            message: network::DseMessageRequest::PlaceBid(bid_recv),
                        )
                        .await;
                }
                Err(_) => {}
            },
            _ => {}
        }
        // self.event_sender.send(event).await.expect("Indexer event message dropped!");
    }
}

pub fn new(
    network_client: network::Client,
    commitment_client: commitment::Client,
    command_receiver: mpsc::Receiver<Command>,
) -> (
    mpsc::Receiver<IndexerEvent>,
    Indexer,
    mpsc::Sender<server::ServerEvent>,
) {
    let (indexer_event_sender, indexer_event_receiver) = mpsc::channel::<IndexerEvent>(10);
    let (server_event_sender, server_event_receeiver) = mpsc::channel::<server::ServerEvent>(10);

    return (
        indexer_event_receiver,
        Indexer::new(
            command_receiver,
            indexer_event_sender,
            server_event_receeiver,
            network_client,
            commitment_client,
        ),
        server_event_sender,
    );
}
