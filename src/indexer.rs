use std::collections::HashMap;

use libp2p::{PeerId, request_response::RequestId};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tokio::{select};

use super::network;
use super::server;

pub type QueryId = [u8; 32];

#[derive(Deserialize, Serialize, Debug)]
pub struct Bid {
    pub query_id: QueryId,
    pub bid: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct BidPlaced {
    // peer id of query requester
    // to whom this bid is placed
    pub requester_id: PeerId,
    pub bid: Bid,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct BidReceived {
    bidder_id: PeerId,
    query_id: QueryId,
    bid: Bid,
}

impl BidReceived {
    pub fn from(bid_placed: BidPlaced, bidder_id: PeerId) -> Self {
        Self {
            bid: bid_placed.bid,
            query_id: bid_placed.bid.query_id,
            bidder_id,
        }
    }
}


#[derive(Deserialize, Serialize, Debug)]
pub struct Query {
    id: QueryId,
    requester_id: PeerId,
    query: String,
    metadata: String,
    expires_at: chrono::DateTime<chrono::Utc>,
}


#[derive(Debug)]
pub enum Command {
    ReceivedBid{
        bid_recv: BidReceived,
        sender: oneshot::Sender<Result<(), anyhow::Error>>,
    }, 
    ReceivedQuery {
        query_recv: Query,
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
    }
}

#[derive(Debug)]
pub enum IndexerEvent {
    SendDseMessageRequest {
        request: network::DseMessageRequest,
        send_to: PeerId,
    }
}


pub struct Client {
    command_sender: mpsc::Sender<Command>,
}

// Client receives commands and forwards them 
impl Client {
    pub async fn handle_received_bid(&mut self, bid_recv: BidReceived) -> Result<(), anyhow::Error> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender.send(
            Command::ReceivedBid { bid_recv, sender}
        ).await.expect("Command message dropped");
        receiver.await.expect("Command response dropped")
    } 

    pub async fn handle_received_query(&mut self, query_recv: Query) -> Result<(), anyhow::Error> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender.send(
            Command::ReceivedQuery { query_recv, sender }
        ).await.expect("Command message dropped");
        receiver.await.expect("Command response dropped")
    }

    pub async fn handle_received_bid_acceptance(&mut self, query_id: QueryId, peer_id: PeerId) -> Result<(), anyhow::Error> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender.send(
            Command::ReceivedBidAcceptance { query_id, peer_id , sender }
        ).await.expect("Command message dropped");
        receiver.await.expect("Command response dropped")
    }

    pub async fn handle_received_start_commit(&mut self, query_id: QueryId, peer_id: PeerId) -> Result<(), anyhow::Error> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender.send(
            Command::ReceivedStartCommit { query_id, peer_id, sender }
        ).await.expect("Command message dropped");
        receiver.await.expect("Command response dropped")
    }
}


/// Main interface thru which user interacts.
/// That means sends and receives querues & bids.
struct Indexer {
    command_receiver: mpsc::Receiver<Command>,
    event_sender: mpsc::Sender<IndexerEvent>,
    server_event_receiver: mpsc::Receiver<server::ServerEvent>,
    server_client_senders: HashMap<usize, mpsc::UnboundedSender<warp::ws::Message>>,
}

impl Indexer {
    pub fn new(
        command_receiver: mpsc::Receiver<Command>,
        event_sender: mpsc::Sender<IndexerEvent>,
        server_event_receiver: mpsc::Receiver<server::ServerEvent>,
    ) -> Self {
        Self {
            command_receiver,
            event_sender,
            server_event_receiver,
            server_client_senders: Default::default(),
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
                // TODO something with the query
                sender.send(Ok(()));
            },
            Command::ReceivedQuery { query_recv, sender } => {  
                // TODO something with query 
                sender.send(Ok(()));
            },
            Command::ReceivedBidAcceptance {query_id, peer_id, sender} => {
                sender.send(Ok(()));
            },
            Command::ReceivedStartCommit {query_id, peer_id, sender} => {
                // notify wallet for commitment
                sender.send(Ok(()));
            }
            _ => {}
        }
    }

    // Indexer events would come from RPC endpoint
    pub async fn server_event_handler(&mut self, event: server::ServerEvent) {
        use server::ServerEvent;
        match event {
            ServerEvent::NewWsClient { client_id, client_sender } => {
                self.server_client_senders.insert(client_id, client_sender);
            },
            ServerEvent::NewWsMessage { client_id, message } => {
                // TODO handle message
            }
        }
        // self.event_sender.send(event).await.expect("Indexer event message dropped!");
    }
}

pub fn new() -> (Client, mpsc::Receiver<IndexerEvent>, Indexer, mpsc::Sender<server::ServerEvent>) {
    let (command_sender, command_receiver) = mpsc::channel::<Command>(10);
    let (indexer_event_sender, indexer_event_receiver) = mpsc::channel::<IndexerEvent>(10);
    let (server_event_sender, server_event_receeiver) = mpsc::channel::<server::ServerEvent>(10);

    return (
        Client {
            command_sender,
        },
        indexer_event_receiver,
        Indexer::new(command_receiver, indexer_event_sender, server_event_receeiver),
        server_event_sender
    )
}


// Note - 
// processing of queries and bids
// should run on different 
// thread (or atleast independent 
// of other things). Therefore, 
// a channel is needed for notifying
// main.

// Node - 
// Create a dummy indexer that sends
// queries/bids randomly.
