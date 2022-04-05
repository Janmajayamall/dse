use libp2p::{request_response, Multiaddr, PeerId};
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::select;
use tokio::sync::{mpsc, oneshot};

use super::commitment;
use super::database;
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

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum BidStatus {
    PendingAcceptance,
    Accepted,
    Service,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub struct BidReceivedWithStatus {
    pub bid_recv: BidReceived,
    pub bid_status: BidStatus,
}

impl BidReceivedWithStatus {
    fn update_status(&mut self, to: BidStatus) {
        self.bid_status = to;
    }
}

impl From<BidReceived> for BidReceivedWithStatus {
    fn from(bid_recv: BidReceived) -> Self {
        Self {
            bid_recv,
            bid_status: BidStatus::PendingAcceptance,
        }
    }
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
    ReceivedRequest {
        request: network::IndexerRequest,
        request_id: request_response::RequestId,
        peer_id: PeerId,
        sender: oneshot::Sender<Result<(), anyhow::Error>>,
    },
    ReceivedServerEvent {
        server_event: server::ServerEvent,
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

    pub async fn handle_received_request(
        &mut self,
        peer_id: PeerId,
        request: network::IndexerRequest,
        request_id: request_response::RequestId,
    ) -> Result<(), anyhow::Error> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::ReceivedRequest {
                request,
                request_id,
                peer_id,
                sender,
            })
            .await
            .expect("Command message dropped");
        receiver.await.expect("Command response dropped")
    }

    pub async fn handle_server_event(
        &mut self,
        server_event: server::ServerEvent,
    ) -> Result<(), anyhow::Error> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::ReceivedServerEvent {
                server_event,
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
    /// Sends events to server clients over websocket
    server_client_senders: HashMap<usize, mpsc::UnboundedSender<server::SendWssMessage>>,
    /// network client
    network_client: network::Client,
    /// commitment client
    commitment_client: commitment::Client,
    /// sent query counter
    query_counter: AtomicUsize,
    /// global database
    database: database::Database,
}

impl Indexer {
    pub fn new(
        command_receiver: mpsc::Receiver<Command>,
        event_sender: mpsc::Sender<IndexerEvent>,

        network_client: network::Client,
        commitment_client: commitment::Client,
        database: database::Database,
    ) -> Self {
        Self {
            command_receiver,
            event_sender,

            server_client_senders: Default::default(),
            network_client,
            commitment_client,
            query_counter: AtomicUsize::new(1),
            database,
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

            }
        }
    }

    // TODO convert indexer command messages to websocket messages
    pub async fn command_handler(&mut self, command: Command) {
        match command {
            Command::ReceivedQuery { query_recv, sender } => {
                self.network_client
                    .add_request_response_peer(
                        query_recv.requester_id.clone(),
                        query_recv.requester_addr.clone(),
                    )
                    .await;

                // save received query
                self.database.insert_received_query(&query_recv);

                // inform server clients
                for (_, client_sender) in self.server_client_senders.iter() {
                    client_sender.send(server::SendWssMessage::ReceivedQuery {
                        query: query_recv.clone(),
                    });
                }
                sender.send(Ok(()));
                debug!("(Command::ReceivedQuery) received query with query_id {:?} from requester {:?} with addr {:?} ", query_recv.id, query_recv.requester_id, query_recv.requester_addr);
            }

            Command::ReceivedRequest {
                request,
                request_id,
                peer_id,
                sender,
            } => {
                match request {
                    network::IndexerRequest::AcceptBid(query_id) => {
                        debug!(
                            "(IndexerRequest::AcceptBid) received bid acceptance for query_id {:?} from requester {:?} ",
                            query_id, peer_id
                        );

                        match self.database.find_user_bid_with_status(&query_id) {
                            Some(mut bid) => {
                                // update bid status to accepted
                                bid.update_status(BidStatus::Accepted);
                                self.database.insert_user_bid_wth_status(&bid);

                                // send ack back to peer
                                self.network_client
                                    .send_dse_message_response(
                                        request_id,
                                        network::DseMessageResponse::Indexer(
                                            network::IndexerResponse::AckAcceptBid(query_id),
                                        ),
                                    )
                                    .await;

                                // flush bid acceptance over ws to client
                                for (_, s) in self.server_client_senders.iter() {
                                    s.send(server::SendWssMessage::ReceivedBidAcceptance {
                                        query_id,
                                    });
                                }

                                sender.send(Ok(()));
                            }
                            None => {
                                debug!(
                                    "(IndexerRequest::AcceptBid) Failed to find bid in user bids for query id {:?}",
                                    query_id
                                );
                                sender.send(Err(anyhow::anyhow!("Failed!")));
                            }
                        }
                    }
                    network::IndexerRequest::PlaceBid(bid_recv) => {
                        let bid_with_status: BidReceivedWithStatus = bid_recv.into();

                        self.network_client
                            .add_request_response_peer(
                                bid_with_status.bid_recv.bidder_id.clone(),
                                bid_with_status.bid_recv.bidder_addr.clone(),
                            )
                            .await;

                        // send ack back to peer
                        self.network_client
                            .send_dse_message_response(
                                request_id,
                                network::DseMessageResponse::Indexer(
                                    network::IndexerResponse::AckBid(
                                        bid_with_status.bid_recv.query_id.clone(),
                                    ),
                                ),
                            )
                            .await;

                        // store bid in db
                        self.database
                            .insert_received_bid_with_status(&bid_with_status);

                        // inform server clients
                        for (_, client_sender) in self.server_client_senders.iter() {
                            client_sender.send(server::SendWssMessage::ReceivedBid {
                                bid: bid_with_status.clone(),
                                query_id: bid_with_status.bid_recv.query_id.clone(),
                            });
                        }

                        debug!(
                            "(IndexerRequest::PlaceBid) received bid for query_id {:?} from bidder {:?} with addr {:?} ",
                            bid_with_status.bid_recv.bid.query_id, bid_with_status.bid_recv.bidder_id, bid_with_status.bid_recv.bidder_addr
                        );
                        sender.send(Ok(()));
                    }
                    network::IndexerRequest::StartCommit(query_id) => {
                        debug!(
                            "(IndexerRequest::StartCommit) received for query_id {:?} from peer {:?} ",
                            query_id, peer_id
                        );

                        match self
                            .database
                            .find_query_bid_with_status(&query_id, &peer_id)
                        {
                            Some(bid) => {
                                // check that bid is on status Accepted
                                if bid.bid_status == BidStatus::Accepted {
                                    // TODO start commitment procedure
                                    // probably notify server clients

                                    // ack start commit to bidder
                                    self.network_client
                                        .send_dse_message_response(
                                            request_id,
                                            network::DseMessageResponse::Indexer(
                                                network::IndexerResponse::AckStartCommit(
                                                    query_id.clone(),
                                                ),
                                            ),
                                        )
                                        .await;

                                    sender.send(Ok(()));
                                } else {
                                    debug!("(IndexerRequest::StartCommit) Bid by bidder id {:?} for query id {:?} hasn't been accepted", peer_id, query_id);
                                    sender.send(Err(anyhow::anyhow!("Failed!")));
                                }
                            }
                            None => {
                                debug!("(IndexerRequest::StartCommit) Bid from bidder {:?} for query id {:?} not found" , peer_id, query_id);
                                sender.send(Err(anyhow::anyhow!("Failed!")));
                            }
                        }
                    }
                }
            }
            Command::ReceivedServerEvent {
                server_event,
                sender,
            } => {
                use server::{ReceivedMessage, ServerEvent};
                match server_event {
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
                    ServerEvent::ReceivedMessage(message) => match message {
                        ReceivedMessage::NewQuery { query } => {
                            if let Ok((peer_id, address)) =
                                self.network_client.network_details().await
                            {
                                debug!("(ServerEvent::ReceivedMessage::NewQuery) received new query from server - {:?} ", query);
                                let id = self.query_counter.fetch_add(1, Ordering::Relaxed);
                                let query_recv = QueryReceived {
                                    id: id.try_into().expect("indexer: Query limit reached"),
                                    requester_id: peer_id,
                                    requester_addr: address,
                                    query,
                                };

                                self.database.insert_user_query(&query_recv);

                                self.network_client
                                    .publish_message(network::GossipsubMessage::NewQuery(
                                        query_recv,
                                    ))
                                    .await;

                                let _ = sender.send(Ok(()));
                            } else {
                            }
                        }
                        ReceivedMessage::PlaceBid { bid } => {
                            if let Ok((node_peer_id, address)) =
                                self.network_client.network_details().await
                            {
                                let bid_recv = BidReceived {
                                    bidder_id: node_peer_id,
                                    bidder_addr: address,
                                    query_id: bid.query_id,
                                    bid: bid.clone(),
                                };

                                match self
                                    .network_client
                                    .send_dse_message_request(
                                        bid_recv.bid.requester_id,
                                        network::DseMessageRequest::Indexer(
                                            network::IndexerRequest::PlaceBid(bid_recv.clone()),
                                        ),
                                    )
                                    .await
                                {
                                    Ok(_) => {
                                        // add bid to database
                                        self.database.insert_user_bid_wth_status(&bid_recv.into());

                                        debug!("(ServerEvent::ReceivedMessage::PlaceBid) Placing bid for query id {:?} to requester id {:?} success", bid.query_id.clone(), bid.requester_id.clone());
                                        sender.send(Ok(()));
                                    }
                                    Err(e) => {
                                        error!("(ServerEvent::ReceivedMessage::PlaceBid) Placing bid for query id {:?} to requester id {:?} failed with error: {:?}", bid.query_id.clone(), bid.requester_id.clone(), e);
                                        sender.send(Err(anyhow::anyhow!("Failed!")));
                                    }
                                }
                            } else {
                            }
                        }
                        ReceivedMessage::AcceptBid {
                            query_id,
                            bidder_id,
                        } => {
                            // check such a bid by bidder for query exists
                            match self
                                .database
                                .find_query_bid_with_status(&query_id, &bidder_id)
                            {
                                Some(mut bid) => {
                                    if bid.bid_recv.query_id == query_id {
                                        match self
                                            .network_client
                                            .send_dse_message_request(
                                                bid.bid_recv.bidder_id,
                                                network::DseMessageRequest::Indexer(
                                                    network::IndexerRequest::AcceptBid(query_id),
                                                ),
                                            )
                                            .await
                                        {
                                            Ok(_) => {
                                                debug!("(ServerEvent::ReceivedMessage::AcceptBid) DSE Accept bid message to bidder id {:?} for query {:?} success", bid.bid_recv.bidder_id.clone(), query_id.clone());
                                                // Update bid status to accepted
                                                // FIXME: change to update afterward,
                                                // to prevent uncessarily adding bid
                                                // when it didn't existed (using API)
                                                bid.update_status(BidStatus::Accepted);
                                                self.database.insert_received_bid_with_status(&bid);

                                                let _ = sender.send(Ok(()));
                                            }
                                            Err(e) => {
                                                error!(
                                                    "(ServerEvent::ReceivedMessage::AcceptBid) DSE Accept Bid request failed with error {:?}",
                                                    e
                                                );
                                                sender.send(Err(anyhow::anyhow!(
                                                    "DSE Accept Bid request failed!"
                                                )));
                                            }
                                        }
                                    } else {
                                        sender.send(Err(anyhow::anyhow!(
                                            "Query Id does not match with Bid's query id"
                                        )));
                                    }
                                }
                                None => {
                                    error!("(ServerEvent::ReceivedMessage::AcceptBid) failed to find bid from bidder id {:?} for query id {:?}", bidder_id, query_id);
                                    sender.send(Err(anyhow::anyhow!(
                                        "Bid with bidder id does not exists"
                                    )));
                                }
                            }
                        }
                        ReceivedMessage::StartCommit { query_id } => {
                            // check user bid exists & is on status Accepted
                            match self.database.find_user_bid_with_status(&query_id) {
                                Some(mut bid) => {
                                    // check status is Accepted
                                    if bid.bid_status == BidStatus::Accepted {
                                        match self
                                            .network_client
                                            .send_dse_message_request(
                                                bid.bid_recv.bid.requester_id,
                                                network::DseMessageRequest::Indexer(
                                                    network::IndexerRequest::StartCommit(
                                                        query_id.clone(),
                                                    ),
                                                ),
                                            )
                                            .await
                                        {
                                            Ok(_) => {
                                                debug!("(ServerEvent::ReceivedMessage::StartCommit) DSE Start commit message to requester id {:?} for query {:?} success", bid.bid_recv.bid.requester_id.clone(), bid.bid_recv.query_id.clone());

                                                // TODO commitment client to start commit
                                                sender.send(Ok(()));
                                            }
                                            Err(e) => {
                                                error!(
                                                    "(ServerEvent::ReceivedMessage::StartCommit) DSE Start commit request failed with error {:?}",
                                                    e
                                                );
                                                sender.send(
                                                    (Err(anyhow::anyhow!(
                                                        "DSE Start commit request failed"
                                                    ))),
                                                );
                                            }
                                        }
                                    } else {
                                        error!("(ServerEvent::ReceivedMessage::StartCommit) Bid for query id {:?} hasn't been accepted",
                                            query_id
                                        );
                                        sender.send(Err(anyhow::anyhow!("Failed!")));
                                    }
                                }
                                None => {
                                    debug!(
                                        "(ServerEvent::ReceivedMessage::StartCommit) Failed to find bid in user bids for query id {:?}",
                                        query_id
                                    );
                                    sender.send(Err(anyhow::anyhow!("Failed!")));
                                }
                            }
                        }
                        _ => {}
                    },
                }
            }
            _ => {}
        }
    }
}

pub fn new(
    network_client: network::Client,
    commitment_client: commitment::Client,
    command_receiver: mpsc::Receiver<Command>,
    database: database::Database,
) -> (mpsc::Receiver<IndexerEvent>, Indexer) {
    let (indexer_event_sender, indexer_event_receiver) = mpsc::channel::<IndexerEvent>(10);

    return (
        indexer_event_receiver,
        Indexer::new(
            command_receiver,
            indexer_event_sender,
            network_client,
            commitment_client,
            database,
        ),
    );
}
