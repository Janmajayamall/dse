use async_std::channel;
use async_std::prelude::StreamExt;
use async_trait::async_trait;
use futures::{AsyncRead, AsyncWrite};
use libp2p::core::connection::ListenerId;
use libp2p::core::either::EitherError;
use libp2p::core::muxing::StreamMuxerBox;
use libp2p::core::transport::upgrade::Version;
use libp2p::core::transport::Boxed;
use libp2p::core::upgrade::{read_length_prefixed, write_length_prefixed, SelectUpgrade};
use libp2p::core::ConnectedPoint;
use libp2p::dns::TokioDnsConfig;
use libp2p::gossipsub::{self, error::GossipsubHandlerError, Gossipsub, GossipsubEvent};
use libp2p::identity::{secp256k1, Keypair};
use libp2p::kad::record::store::MemoryStore;
use libp2p::kad::record::Key;
use libp2p::kad::{
    Addresses, BootstrapError, GetRecordError, GetRecordOk, Kademlia, KademliaConfig,
    KademliaEvent, PutRecordOk, QueryId, QueryResult, Quorum, Record,
};
use libp2p::mdns::{Mdns, MdnsConfig, MdnsEvent};
use libp2p::mplex::MplexConfig;
use libp2p::noise;
use libp2p::ping::{self, Ping, PingConfig, PingEvent};
use libp2p::request_response::{
    self, ProtocolSupport, RequestResponse, RequestResponseCodec, RequestResponseEvent,
    RequestResponseMessage,
};
use libp2p::swarm::{ConnectionHandlerUpgrErr, SwarmBuilder, SwarmEvent};
use libp2p::tcp::TokioTcpConfig;
use libp2p::yamux::YamuxConfig;
use libp2p::{Multiaddr, NetworkBehaviour, PeerId, Swarm, Transport};
use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::error::Error;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::{io, select};

use super::commitment;
use super::indexer;

#[derive(NetworkBehaviour)]
#[behaviour(out_event = "BehaviourEvent")]
struct Behaviour {
    kademlia: Kademlia<MemoryStore>,
    mdns: Mdns,
    ping: Ping,
    gossipsub: Gossipsub,
    request_response: RequestResponse<DseMessageCodec>,
}

impl Behaviour {
    pub async fn new(keypair: &Keypair) -> Result<Self, anyhow::Error> {
        let peer_id = keypair.public().to_peer_id();
        // setup kademlia
        let store = MemoryStore::new(peer_id);
        let mut kad_config = KademliaConfig::default();
        kad_config.set_protocol_name("/dse/kad/1.0.0".as_bytes());
        kad_config.set_query_timeout(Duration::from_secs(300));
        // set disjoint_query_paths to true. Ref: https://discuss.libp2p.io/t/s-kademlia-lookups-over-disjoint-paths-in-rust-libp2p/571
        kad_config.disjoint_query_paths(true);
        let kademlia = Kademlia::with_config(peer_id, store, kad_config);

        // mdns
        let mdns = Mdns::new(MdnsConfig::default()).await.unwrap();

        // ping
        let ping_config = PingConfig::new().with_keep_alive(true);
        let ping = Ping::new(ping_config);

        // gossipsub
        // TODO change gossipsub protocol identifier from default to something usable
        let gossipsub_config = gossipsub::GossipsubConfigBuilder::default()
            .validation_mode(gossipsub::ValidationMode::Strict)
            .build()
            .map_err(anyhow::Error::msg)?;
        let mut gossipsub = Gossipsub::new(
            gossipsub::MessageAuthenticity::Signed(keypair.clone()),
            gossipsub_config,
        )
        .map_err(anyhow::Error::msg)?;

        // by default subcribe to search query topic
        gossipsub.subscribe(&GossipsubTopic::Query.ident_topic())?;
        debug!("gosipsub subscribed");

        // request response
        let request_response = RequestResponse::new(
            DseMessageCodec(),
            std::iter::once((DseMessageProtocol(), ProtocolSupport::Full)),
            Default::default(),
        );

        Ok(Behaviour {
            kademlia,
            mdns,
            ping,
            gossipsub,
            request_response,
        })
    }
}

pub enum BehaviourEvent {
    Kademlia(KademliaEvent),
    Ping(PingEvent),
    Mdns(MdnsEvent),
    Gossipsub(GossipsubEvent),
    RequestResponse(RequestResponseEvent<DseMessageRequest, DseMessageResponse>),
}

impl From<KademliaEvent> for BehaviourEvent {
    fn from(event: KademliaEvent) -> Self {
        BehaviourEvent::Kademlia(event)
    }
}

impl From<PingEvent> for BehaviourEvent {
    fn from(event: PingEvent) -> Self {
        BehaviourEvent::Ping(event)
    }
}

impl From<MdnsEvent> for BehaviourEvent {
    fn from(event: MdnsEvent) -> Self {
        BehaviourEvent::Mdns(event)
    }
}

impl From<GossipsubEvent> for BehaviourEvent {
    fn from(event: GossipsubEvent) -> Self {
        BehaviourEvent::Gossipsub(event)
    }
}

impl From<RequestResponseEvent<DseMessageRequest, DseMessageResponse>> for BehaviourEvent {
    fn from(event: RequestResponseEvent<DseMessageRequest, DseMessageResponse>) -> Self {
        BehaviourEvent::RequestResponse(event)
    }
}

pub struct CustomExecutor;
impl libp2p::core::Executor for CustomExecutor {
    fn exec(
        &self,
        future: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'static + Send>>,
    ) {
        tokio::task::spawn(future);
    }
}

/// Creates a new network interface. Also
/// sets up network event stream
// pub async fn new(
//     seed: Option<u8>,
//     command_receiver: mpsc::Receiver<Command>,
// ) -> Result<(mpsc::Receiver<NetworkEvent>, Network), Box<dyn Error>> {
//     // FIXME: keypair generation should happen in daemon
//     let keypair = match seed {
//         Some(seed) => {
//             let mut bytes = [0u8; 32];
//             bytes[0] = seed;
//             let secret_key = secp256k1::SecretKey::from_bytes(bytes).expect("Invalid seed");
//             Keypair::Secp256k1(secret_key.into())
//         }
//         None => Keypair::generate_secp256k1(),
//     };
//     let peer_id = keypair.public().to_peer_id();
//     debug!("Node peer id {:?} ", peer_id.to_base58());

//     // Build swarm
//     let transport = build_transport(&keypair)?;
//     let behaviour = Behaviour::new(&keypair).await;
//     let swarm = SwarmBuilder::new(transport, behaviour, peer_id)
//         .executor(Box::new(CustomExecutor))
//         .build();

//     let (network_event_sender, network_event_receiver) = mpsc::channel::<NetworkEvent>(10);

//     // network interface
//     let network_interface = Network::new(keypair, swarm, command_receiver, network_event_sender);

//     Ok((network_event_receiver, network_interface))
// }

/// Uses TCP encrypted using noise DH and MPlex for multiplexing
pub fn build_transport(identity_keypair: &Keypair) -> io::Result<Boxed<(PeerId, StreamMuxerBox)>> {
    // noise config
    let keypair = noise::Keypair::<noise::X25519>::new()
        .into_authentic(identity_keypair)
        .unwrap();
    let noise_config = noise::NoiseConfig::xx(keypair).into_authenticated();

    Ok(TokioDnsConfig::system(TokioTcpConfig::new())?
        .upgrade(Version::V1)
        .authenticate(noise_config)
        .multiplex(SelectUpgrade::new(
            YamuxConfig::default(),
            MplexConfig::new(),
        ))
        .timeout(Duration::from_secs(20))
        .map(|(peer_id, muxer), _| (peer_id, StreamMuxerBox::new(muxer)))
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err))
        .boxed())
}

pub struct Network {
    /// keypair of the node
    pub keypair: Keypair,
    /// Multiaddr of the node
    pub node_address: Option<Multiaddr>,

    swarm: Swarm<Behaviour>,

    command_sender: mpsc::Sender<Command>,
    command_receiver: mpsc::Receiver<Command>,
    network_event_sender: channel::Sender<NetworkEvent>,
    network_event_receiver: channel::Receiver<NetworkEvent>,

    pending_dht_put_requests: HashMap<QueryId, oneshot::Sender<Result<PutRecordOk, anyhow::Error>>>,
    pending_dht_get_requests: HashMap<QueryId, oneshot::Sender<Result<GetRecordOk, anyhow::Error>>>,
    pending_add_peers: HashMap<PeerId, oneshot::Sender<Result<(), anyhow::Error>>>,
    pending_bootstrap_requests: HashMap<QueryId, oneshot::Sender<Result<(), anyhow::Error>>>,
    // Pending result of outbound request. It returns DseMessageResponse
    // which means the fn does not finishes until peer to which outcound request
    // is sent, responds using channel or channel drops.
    pending_dse_outbound_message_requests: HashMap<
        request_response::RequestId,
        oneshot::Sender<Result<DseMessageResponse, anyhow::Error>>,
    >,
    // Pending result of response sent to an inbound request
    pending_dse_inbound_message_response:
        HashMap<request_response::RequestId, oneshot::Sender<Result<(), anyhow::Error>>>,
    // Stores response channels of an inbound request
    response_channels_for_inbound_requests:
        HashMap<request_response::RequestId, request_response::ResponseChannel<DseMessageResponse>>,
    // All known peers (FIX: Not needed anymore?)
    known_peers: HashMap<PeerId, Addresses>,
}

impl Network {
    pub async fn new(keypair: Keypair) -> Result<Self, anyhow::Error> {
        // Build swarm
        let transport = build_transport(&keypair)?;
        let behaviour = Behaviour::new(&keypair).await?;
        let swarm = SwarmBuilder::new(transport, behaviour, keypair.public().to_peer_id())
            .executor(Box::new(CustomExecutor))
            .build();

        let (command_sender, command_receiver) = mpsc::channel(10);
        let (network_event_sender, network_event_receiver) = channel::unbounded();

        Ok(Self {
            keypair,
            node_address: None,
            swarm,

            command_sender,
            command_receiver,
            network_event_sender,
            network_event_receiver,

            pending_dht_put_requests: Default::default(),
            pending_dht_get_requests: Default::default(),
            pending_add_peers: Default::default(),
            pending_bootstrap_requests: Default::default(),
            pending_dse_outbound_message_requests: Default::default(),
            pending_dse_inbound_message_response: Default::default(),
            response_channels_for_inbound_requests: Default::default(),
            known_peers: Default::default(),
        })
    }

    pub async fn run(mut self) {
        loop {
            select! {
                command = self.command_receiver.recv() => {
                    match command {
                        Some(val) => {
                            self.command_handler(val).await
                        },
                        None => {return},
                    }
                }
                swarm_event = self.swarm.next() => {
                    match swarm_event {
                        Some(event) => {
                            self.swarm_event_handler(event).await;
                        }
                        None => {return}
                    }
                }
            }
        }
    }

    fn network_event_receiver(&self) -> channel::Receiver<NetworkEvent> {
        self.network_event_receiver.clone()
    }

    fn network_command_sender(&self) -> mpsc::Sender<Command> {
        self.command_sender.clone()
    }

    async fn command_handler(&mut self, command: Command) {
        match command {
            Command::StartListening { addr, sender } => match self.swarm.listen_on(addr) {
                Ok(_) => {
                    sender.send(Ok(()));
                }
                Err(e) => {
                    sender.send(Err(e.into()));
                }
            },
            Command::AddKadPeer {
                peer_id,
                peer_addr,
                sender,
            } => {
                self.swarm
                    .behaviour_mut()
                    .kademlia
                    .add_address(&peer_id, peer_addr.clone());
                let _ = sender.send(());
                debug!(
                    "(kad) peer added with peerId {:?} multiAddr {:?}",
                    peer_id, peer_addr
                );
            }
            Command::AddRequestResponsePeer {
                peer_id,
                peer_addr,
                sender,
            } => {
                self.swarm
                    .behaviour_mut()
                    .request_response
                    .add_address(&peer_id, peer_addr.clone());
                let _ = sender.send(());
                debug!(
                    "(request_response) peer added with peerId {:?} multiAddr {:?}",
                    peer_id, peer_addr
                );
            }
            Command::DhtPut {
                record,
                quorum,
                sender,
            } => {
                match self
                    .swarm
                    .behaviour_mut()
                    .kademlia
                    .put_record(record.clone(), quorum)
                {
                    Ok(query_id) => {
                        self.pending_dht_put_requests.insert(query_id, sender);
                    }
                    Err(e) => {
                        sender.send(Err(e.into()));
                    }
                }
            }
            Command::DhtGet {
                key,
                quorum,
                sender,
            } => {
                let query_id = self.swarm.behaviour_mut().kademlia.get_record(key, quorum);
                self.pending_dht_get_requests.insert(query_id, sender);
            }
            Command::Bootstrap { sender } => {
                match self.swarm.behaviour_mut().kademlia.bootstrap() {
                    Ok(q) => {
                        self.pending_bootstrap_requests.insert(q, sender);
                    }
                    Err(e) => {
                        sender.send(Err(e.into()));
                    }
                }
            }

            Command::PublishMessage { message, sender } => {
                match self
                    .swarm
                    .behaviour_mut()
                    .gossipsub
                    .publish(message.topic(), message.clone())
                {
                    Ok(m_id) => {
                        debug!("(gossipsub) published a message {:?}", message);
                        sender.send(Ok(m_id));
                    }
                    Err(e) => {
                        error!("(gossipsub) failed to publish message {:?}", e);
                        sender.send(Err(e.into()));
                    }
                }
            }
            Command::SendDseMessageResponse {
                request_id,
                response,
                sender,
            } => {
                match self
                    .response_channels_for_inbound_requests
                    .remove(&request_id)
                {
                    Some(channel) => {
                        self.swarm
                            .behaviour_mut()
                            .request_response
                            .send_response(channel, response);
                        self.pending_dse_inbound_message_response
                            .insert(request_id, sender);
                    }
                    None => {
                        sender.send(Err(anyhow::anyhow!(
                            "Response channel is missing for inboud request with requst id: {}",
                            request_id
                        )));
                    }
                }
            }
            Command::SendDseMessageRequest {
                peer_id,
                message,
                sender,
            } => {
                let request_id = self
                    .swarm
                    .behaviour_mut()
                    .request_response
                    .send_request(&peer_id, message);
                self.pending_dse_outbound_message_requests
                    .insert(request_id, sender);
            }
            Command::NetworkDetails { sender } => match self.node_address.clone() {
                Some(v) => {
                    sender.send(Ok((self.keypair.public().to_peer_id(), v)));
                }
                None => {
                    sender.send(Err(anyhow::anyhow!(
                        "(network) node does not have local address assigned"
                    )));
                }
            },
        }
    }

    async fn swarm_event_handler(
        &mut self,
        event: SwarmEvent<
            BehaviourEvent,
            EitherError<
                EitherError<
                    EitherError<EitherError<std::io::Error, void::Void>, ping::Failure>,
                    GossipsubHandlerError,
                >,
                ConnectionHandlerUpgrErr<std::io::Error>,
            >,
        >,
    ) {
        match event {
            SwarmEvent::Behaviour(BehaviourEvent::Mdns(event)) => {
                emit_event(&self.network_event_sender, NetworkEvent::Mdns(event)).await;
            }
            SwarmEvent::Behaviour(BehaviourEvent::Kademlia(
                KademliaEvent::OutboundQueryCompleted { id, result, .. },
            )) => match result {
                QueryResult::PutRecord(Ok(record)) => {
                    if let Some(sender) = self.pending_dht_put_requests.remove(&id) {
                        sender.send(Ok(record));
                    } else {
                        emit_response_channel_missing(&id);
                    }
                }
                QueryResult::PutRecord(Err(err)) => {
                    if let Some(sender) = self.pending_dht_put_requests.remove(&id) {
                        sender.send(Err(err.into()));
                    } else {
                        emit_response_channel_missing(&id);
                    }
                }
                QueryResult::GetRecord(Ok(record)) => {
                    if let Some(sender) = self.pending_dht_get_requests.remove(&id) {
                        sender.send(Ok(record));
                    } else {
                        emit_response_channel_missing(&id);
                    }
                }
                QueryResult::GetRecord(Err(err)) => {
                    if let Some(sender) = self.pending_dht_get_requests.remove(&id) {
                        // FIXME: GetRecordError doen not implement
                        // Error trait. Track issue here - https://github.com/libp2p/rust-libp2p/issues/2612.
                        // sender.send(Err(err.into()));
                        sender.send(Err(anyhow::anyhow!("Get record error")));
                    } else {
                        emit_response_channel_missing(&id);
                    }
                }
                QueryResult::Bootstrap(Ok(record)) => {
                    debug!(
                        "kad: Bootstrap with peer id {:?} success; num remaining {:?} ",
                        record.peer, record.num_remaining
                    );
                    if record.num_remaining == 0 {
                        if let Some(sender) = self.pending_bootstrap_requests.remove(&id) {
                            sender.send(Ok(()));
                        } else {
                            emit_response_channel_missing(&id);
                        }
                    }
                }
                QueryResult::Bootstrap(Err(err)) => match err {
                    BootstrapError::Timeout {
                        peer,
                        num_remaining,
                    } => {
                        error!(
                            "kad: Bootstrap with peer id {:?} failed; num remaining {:?} ",
                            peer, num_remaining
                        );

                        if num_remaining.is_none() || num_remaining.unwrap() == 0 {
                            if let Some(sender) = self.pending_bootstrap_requests.remove(&id) {
                                sender.send(Err(err.into()));
                            } else {
                                emit_response_channel_missing(&id);
                            }
                        }
                    }
                },
                _ => return,
            },
            SwarmEvent::Behaviour(BehaviourEvent::Kademlia(KademliaEvent::RoutingUpdated {
                peer,
                addresses,
                ..
            })) => {
                debug!(
                    "(kad) routing updated with peerId {:?} address {:?} ",
                    peer, addresses
                );

                // TODO: I haven't taken care of eviction of old peers here
                // self.known_peers.insert(peer, addresses);
            }
            SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(GossipsubEvent::Message {
                message,
                ..
            })) => {
                debug!("(gossipsub) Received message {:?} ", message.clone());
                match GossipsubMessage::try_from(message) {
                    Ok(m) => {
                        emit_event(
                            &self.network_event_sender,
                            NetworkEvent::GossipsubMessageRecv(m),
                        )
                        .await;
                    }
                    Err(e) => {
                        error!("(gossipsub) Received message errored with {}", e);
                    }
                }
            }
            SwarmEvent::Behaviour(BehaviourEvent::RequestResponse(
                RequestResponseEvent::Message { peer, message },
            )) => {
                match message {
                    RequestResponseMessage::Request {
                        request_id,
                        request,
                        channel,
                    } => {
                        // debug!("request_response: Received request {:?} ", request.clone());
                        self.response_channels_for_inbound_requests
                            .insert(request_id, channel);
                        emit_event(
                            &self.network_event_sender,
                            NetworkEvent::DseMessageRequestRecv {
                                peer_id: peer,
                                request_id,
                                request,
                            },
                        )
                        .await;
                    }
                    RequestResponseMessage::Response {
                        request_id,
                        response,
                    } => {
                        // debug!(
                        //     "request_response: Received response {:?} ",
                        //     response.clone()
                        // );
                        if let Some(sender) = self
                            .pending_dse_outbound_message_requests
                            .remove(&request_id)
                        {
                            sender.send(Ok(response));
                        } else {
                            error!("(Outbound message request) response channel missing for request id {}", request_id);
                        }
                    }
                }
            }
            SwarmEvent::Behaviour(BehaviourEvent::RequestResponse(
                RequestResponseEvent::ResponseSent { peer, request_id },
            )) => {
                if let Some(sender) = self
                    .pending_dse_inbound_message_response
                    .remove(&request_id)
                {
                    sender.send(Ok(()));
                } else {
                    error!(
                        "(Inbound message response) response channel missing for request id {}",
                        request_id
                    );
                }
            }
            SwarmEvent::Behaviour(BehaviourEvent::RequestResponse(
                RequestResponseEvent::OutboundFailure {
                    peer,
                    request_id,
                    error,
                },
            )) => {
                if let Some(sender) = self
                    .pending_dse_outbound_message_requests
                    .remove(&request_id)
                {
                    sender.send(Err(error.into()));
                } else {
                    error!(
                        "(Outbound message request) response channel missing for request id {}",
                        request_id
                    );
                }
            }
            SwarmEvent::Behaviour(BehaviourEvent::RequestResponse(
                RequestResponseEvent::InboundFailure {
                    peer,
                    request_id,
                    error,
                },
            )) => {
                if let Some(sender) = self
                    .pending_dse_inbound_message_response
                    .remove(&request_id)
                {
                    sender.send(Err(error.into()));
                } else {
                    error!(
                        "(Inbound message response) response channel missing for request id {}",
                        request_id
                    );
                }
            }
            SwarmEvent::IncomingConnection {
                local_addr,
                send_back_addr,
            } => {
                debug!(
                    "swarm: incoming connection {:?} {:?} ",
                    local_addr, send_back_addr
                );
            }
            SwarmEvent::ConnectionEstablished {
                peer_id,
                endpoint,
                num_established,
                ..
            } => {
                debug!(
                    "swarm: connection established {:?} {:?} {:?} ",
                    peer_id, endpoint, num_established
                );
                match endpoint {
                    // Not sure whether this is the right practice for populating
                    // kad dht routing table of a node.
                    // At present, this is only needed for a bootstrap node since it needs
                    // to maintain an uptodate view of kad routing table to facilitate bootstrapping
                    // of new nodes to the network.
                    // We can skip this by a separate request that new node sends to be added
                    // to bootstrap node's routing table.
                    ConnectedPoint::Listener { send_back_addr, .. } => {
                        if !self.known_peers.contains_key(&peer_id) {
                            self.swarm
                                .behaviour_mut()
                                .kademlia
                                .add_address(&peer_id, send_back_addr.clone());
                        }
                    }
                    _ => {}
                }
            }
            SwarmEvent::ConnectionClosed {
                peer_id,
                endpoint,
                num_established,
                cause,
            } => {
                debug!(
                    "swarm: connection closed {:?} {:?} {:?} {:?} ",
                    peer_id, endpoint, num_established, cause
                );
            }
            SwarmEvent::NewListenAddr {
                listener_id,
                address,
            } => {
                debug!(
                    "swarm: new listener id {:?} and addr {:?} ",
                    listener_id, address
                );
                self.node_address = Some(address);
                // self.network_event_sender.send(NetworkEvent::NewListenAddr { listener_id, address}).await.expect("Network evvent message dropped");
            }
            _ => {}
        }
    }
}

async fn emit_event(sender: &channel::Sender<NetworkEvent>, event: NetworkEvent) {
    if sender.send(event).await.is_err() {
        error!("Network evnent failed: Network event receiver dropped");
    }
}

fn emit_response_channel_missing(query_id: &QueryId) {
    error!(
        "Response channel for pending req QueryId {:?} missing",
        query_id
    );
}

#[derive(Debug)]
pub enum Command {
    StartListening {
        addr: Multiaddr,
        sender: oneshot::Sender<Result<(), anyhow::Error>>,
    },
    AddKadPeer {
        peer_id: PeerId,
        peer_addr: Multiaddr,
        sender: oneshot::Sender<()>,
    },
    AddRequestResponsePeer {
        peer_id: PeerId,
        peer_addr: Multiaddr,
        sender: oneshot::Sender<()>,
    },
    DhtPut {
        record: Record,
        quorum: Quorum,
        sender: oneshot::Sender<Result<PutRecordOk, anyhow::Error>>,
    },
    DhtGet {
        key: Key,
        quorum: Quorum,
        sender: oneshot::Sender<Result<GetRecordOk, anyhow::Error>>,
    },
    Bootstrap {
        sender: oneshot::Sender<Result<(), anyhow::Error>>,
    },
    PublishMessage {
        message: GossipsubMessage,
        sender: oneshot::Sender<Result<gossipsub::MessageId, anyhow::Error>>,
    },
    SendDseMessageResponse {
        request_id: request_response::RequestId,
        response: DseMessageResponse,
        sender: oneshot::Sender<Result<(), anyhow::Error>>,
    },
    SendDseMessageRequest {
        peer_id: PeerId,
        message: DseMessageRequest,
        sender: oneshot::Sender<Result<DseMessageResponse, anyhow::Error>>,
    },
    NetworkDetails {
        sender: oneshot::Sender<Result<(PeerId, Multiaddr), anyhow::Error>>,
    },
}

#[derive(Debug)]
pub enum NetworkEvent {
    Mdns(MdnsEvent),
    GossipsubMessageRecv(GossipsubMessage),
    GossipsubMessageRecvErr(Box<dyn Error + Send>),
    DseMessageRequestRecv {
        peer_id: PeerId,
        request_id: request_response::RequestId,
        request: DseMessageRequest,
    },
    NewListenAddr {
        listener_id: ListenerId,
        address: Multiaddr,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum GossipsubMessage {
    NewQuery(indexer::QueryReceived),
}

impl TryFrom<gossipsub::GossipsubMessage> for GossipsubMessage {
    type Error = Box<dyn std::error::Error + Send + Sync>;
    fn try_from(recv_message: gossipsub::GossipsubMessage) -> Result<Self, Self::Error> {
        match recv_message.source {
            Some(peer_id) => match serde_json::from_slice(&recv_message.data) {
                Ok(v) => Ok(v),
                Err(e) => Err(Box::new(e)),
            },
            None => Err("gossipsub: Gossipsub message does not contain source peer id".into()),
        }
    }
}

impl Into<Vec<u8>> for GossipsubMessage {
    fn into(self) -> Vec<u8> {
        match serde_json::to_vec(&self) {
            Ok(v) => v,
            Err(_) => Default::default(),
        }
    }
}

impl GossipsubMessage {
    fn topic(&self) -> gossipsub::IdentTopic {
        GossipsubTopic::from_message(self).ident_topic()
    }
}

#[derive(Debug)]
pub enum GossipsubTopic {
    Query,
}

impl GossipsubTopic {
    fn ident_topic(self) -> gossipsub::IdentTopic {
        match self {
            GossipsubTopic::Query => gossipsub::IdentTopic::new("Query"),
        }
    }

    fn from_message(message: &GossipsubMessage) -> Self {
        match message {
            GossipsubMessage::NewQuery { .. } => GossipsubTopic::Query,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum CommitRequest {
    /// Ask for wallet address
    WalletAddress(indexer::QueryId),
    /// Ask for commitment
    CommitFund {
        query_id: indexer::QueryId,
        round: u32,
    },
    /// Invalidating Signature
    /// Sent by service requester
    InvalidatingSignature {
        query_id: indexer::QueryId,
        invalidating_signature: ethers::types::Signature,
    },
    /// End commit procedure
    EndCommit(indexer::QueryId),
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum CommitResponse {
    /// Wallet address response
    WalletAddress {
        query_id: indexer::QueryId,
        wallet_address: ethers::types::Address,
    },
    /// CommitFund response
    CommitFund {
        query_id: indexer::QueryId,
        round: u32,
        commitment: commitment::Commit,
    },
    /// Ack InvalidatingSignature
    AckInvalidatingSignature(indexer::QueryId),
    /// Ack End Commit
    AckEndCommit(indexer::QueryId),
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum IndexerRequest {
    /// Place bid for a query
    PlaceBid(indexer::BidReceived),
    /// Accept a bid for a query
    AcceptBid(indexer::QueryId),
    /// Start commit procedure
    StartCommit(indexer::QueryId),
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum IndexerResponse {
    /// Ack that bid was received
    AckBid(indexer::QueryId),
    /// Ack that bid acceptance was received
    AckAcceptBid(indexer::QueryId),
    /// Ack that start commit was received
    AckStartCommit(indexer::QueryId),
}

// All stuff related to request response
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum DseMessageRequest {
    /// Requests related to indexer
    Indexer(IndexerRequest),
    /// Requests related to commit fund
    Commit(CommitRequest),
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum DseMessageResponse {
    /// Responses related to indexer
    Indexer(IndexerResponse),
    /// Responss related to commit fund
    Commit(CommitResponse),
}

#[derive(Debug, Clone)]
pub struct DseMessageProtocol();

impl request_response::ProtocolName for DseMessageProtocol {
    fn protocol_name(&self) -> &[u8] {
        b"/dse-message/1"
    }
}

#[derive(Clone)]
struct DseMessageCodec();

#[async_trait]
impl RequestResponseCodec for DseMessageCodec {
    type Protocol = DseMessageProtocol;
    type Request = DseMessageRequest;
    type Response = DseMessageResponse;

    async fn read_request<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        let vec = read_length_prefixed(io, 10000000).await?;
        match serde_json::from_slice(&vec) {
            Ok(v) => Ok(v),
            Err(e) => Err(io::ErrorKind::Other.into()),
        }
    }

    async fn read_response<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        let vec = read_length_prefixed(io, 10000000).await?;
        match serde_json::from_slice(&vec) {
            Ok(v) => Ok(v),
            Err(_) => Err(io::ErrorKind::Other.into()),
        }
    }

    async fn write_request<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: &mut T,
        req: Self::Request,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        match serde_json::to_vec(&req) {
            Ok(val) => {
                write_length_prefixed(io, &val).await?;
                Ok(())
            }
            Err(_) => Err(io::ErrorKind::Other.into()),
        }
    }

    async fn write_response<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: &mut T,
        res: Self::Response,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        match serde_json::to_vec(&res) {
            Ok(val) => {
                write_length_prefixed(io, &val).await?;
                Ok(())
            }
            Err(_) => Err(io::ErrorKind::Other.into()),
        }
    }
}
