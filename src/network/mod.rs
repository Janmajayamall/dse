use super::storage;
use async_std::prelude::StreamExt;
use async_trait::async_trait;
use ethers::types::{Address, Signature};
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
use libp2p::identity::Keypair;
use libp2p::kad::record::store::MemoryStore;
use libp2p::kad::record::Key;
use libp2p::kad::{
    AddProviderOk, Addresses, BootstrapError, GetProvidersOk, GetRecordOk, Kademlia,
    KademliaConfig, KademliaEvent, KademliaStoreInserts, PutRecordOk, QueryId, QueryResult, Quorum,
    Record,
};
use libp2p::mdns::{Mdns, MdnsConfig, MdnsEvent};
use libp2p::mplex::MplexConfig;
use libp2p::noise;
use libp2p::ping::{self, Ping, PingConfig, PingEvent};
use libp2p::request_response::{
    ProtocolSupport, RequestId, RequestResponse, RequestResponseCodec, RequestResponseEvent,
    RequestResponseMessage, ResponseChannel,
};
use libp2p::swarm::{ConnectionHandlerUpgrErr, SwarmBuilder, SwarmEvent};
use libp2p::tcp::TokioTcpConfig;
use libp2p::yamux::YamuxConfig;
use libp2p::{Multiaddr, NetworkBehaviour, PeerId, Swarm, Transport};
use log::{debug, error, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::{io, select};

mod commit;
mod exchange;
pub use commit::*;
pub use exchange::*;
mod request_response;

#[derive(NetworkBehaviour)]
#[behaviour(out_event = "BehaviourEvent")]
struct Behaviour {
    kademlia: Kademlia<MemoryStore>,
    mdns: Mdns,
    ping: Ping,
    gossipsub: Gossipsub,
    exchange: RequestResponse<ExchangeCodec>,
    commit: RequestResponse<CommitCodec>,
}

impl Behaviour {
    pub async fn new(keypair: &Keypair) -> Result<Self, anyhow::Error> {
        let peer_id = keypair.public().to_peer_id();
        // setup kademlia
        let store = MemoryStore::new(peer_id);
        let mut kad_config = KademliaConfig::default();
        kad_config.set_protocol_name("/dse/kad/1.0.0".as_bytes());
        kad_config.set_query_timeout(Duration::from_secs(300));
        kad_config.set_record_filtering(KademliaStoreInserts::FilterBoth);
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
        let gossipsub = Gossipsub::new(
            gossipsub::MessageAuthenticity::Signed(keypair.clone()),
            gossipsub_config,
        )
        .map_err(anyhow::Error::msg)?;

        let exchange = RequestResponse::new(
            ExchangeCodec::default(),
            std::iter::once((ExchangeProtocol, ProtocolSupport::Full)),
            Default::default(),
        );

        let commit = RequestResponse::new(
            CommitCodec::default(),
            std::iter::once((CommitProtocol, ProtocolSupport::Full)),
            Default::default(),
        );

        Ok(Behaviour {
            kademlia,
            mdns,
            ping,
            gossipsub,

            exchange,
            commit,
        })
    }
}

pub enum BehaviourEvent {
    Kademlia(KademliaEvent),
    Ping(PingEvent),
    Mdns(MdnsEvent),
    Gossipsub(GossipsubEvent),

    Exchange(RequestResponseEvent<ExchangeRequest, ExchangeResponse>),
    Commit(RequestResponseEvent<CommitRequest, CommitResponse>),
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

impl From<RequestResponseEvent<ExchangeRequest, ExchangeResponse>> for BehaviourEvent {
    fn from(event: RequestResponseEvent<ExchangeRequest, ExchangeResponse>) -> Self {
        BehaviourEvent::Exchange(event)
    }
}

impl From<RequestResponseEvent<CommitRequest, CommitResponse>> for BehaviourEvent {
    fn from(event: RequestResponseEvent<CommitRequest, CommitResponse>) -> Self {
        BehaviourEvent::Commit(event)
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
    network_event_sender: broadcast::Sender<NetworkEvent>,
    network_event_receiver: broadcast::Receiver<NetworkEvent>,

    pending_dht_put_requests: HashMap<QueryId, oneshot::Sender<Result<PutRecordOk, anyhow::Error>>>,
    pending_dht_start_providing_requests:
        HashMap<QueryId, oneshot::Sender<Result<AddProviderOk, anyhow::Error>>>,
    pending_dht_get_requests: HashMap<QueryId, oneshot::Sender<Result<GetRecordOk, anyhow::Error>>>,
    pending_dht_get_providers_requests:
        HashMap<QueryId, oneshot::Sender<Result<GetProvidersOk, anyhow::Error>>>,
    pending_add_peers: HashMap<PeerId, oneshot::Sender<Result<(), anyhow::Error>>>,
    pending_bootstrap_requests: HashMap<QueryId, oneshot::Sender<Result<(), anyhow::Error>>>,

    pending_exchange_outbound_requests:
        HashMap<(PeerId, RequestId), oneshot::Sender<Result<ExchangeResponse, anyhow::Error>>>,
    pending_commit_outbound_requests:
        HashMap<(PeerId, RequestId), oneshot::Sender<Result<CommitResponse, anyhow::Error>>>,

    pending_exchange_inbound_response:
        HashMap<(PeerId, RequestId), oneshot::Sender<Result<(), anyhow::Error>>>,
    pending_commit_inbound_response:
        HashMap<(PeerId, RequestId), oneshot::Sender<Result<(), anyhow::Error>>>,

    exchange_inbound_response_channels:
        HashMap<(PeerId, RequestId), ResponseChannel<ExchangeResponse>>,
    commit_inbound_response_channels: HashMap<(PeerId, RequestId), ResponseChannel<CommitResponse>>,

    // All known peers (FIX: Not needed anymore?)
    known_peers: HashMap<PeerId, Addresses>,
}

impl Network {
    pub async fn new(
        keypair: Keypair,
        bootstrap_peers: Vec<(PeerId, Multiaddr)>,
        listen_on: Multiaddr,
    ) -> Result<Self, anyhow::Error> {
        // Build swarm
        let transport = build_transport(&keypair)?;
        let behaviour = Behaviour::new(&keypair).await?;
        let mut swarm = SwarmBuilder::new(transport, behaviour, keypair.public().to_peer_id())
            .executor(Box::new(CustomExecutor))
            .build();

        // subscribe to search query topic
        swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&GossipsubTopic::Query.ident_topic())?;
        debug!("gosipsub subscribed ");

        // Add bootstrap peers to kad routing table
        for (peer_id, addr) in bootstrap_peers {
            swarm
                .behaviour_mut()
                .kademlia
                .add_address(&peer_id, addr.clone());

            // dial in
            if let Err(e) = swarm.dial(addr.clone()) {
                warn!(
                    "Dial in failed for peer_id {} at address {} with error :{}",
                    peer_id, addr, e
                );
            }
        }

        // Bootstrap
        if let Err(e) = swarm.behaviour_mut().kademlia.bootstrap() {
            warn!("Failed to bootstrap node with error {}", e);
        }

        // Start listening on default addr
        if let Err(e) = swarm.listen_on(listen_on) {
            error!(
                "Failed to start listening on default address with error {}",
                e
            );
        }

        let (command_sender, command_receiver) = mpsc::channel(10);
        let (network_event_sender, network_event_receiver) = broadcast::channel(20);

        Ok(Self {
            keypair,
            node_address: None,
            swarm,

            command_sender,
            command_receiver,
            network_event_sender,
            network_event_receiver,

            pending_dht_put_requests: Default::default(),
            pending_dht_start_providing_requests: Default::default(),
            pending_dht_get_requests: Default::default(),
            pending_dht_get_providers_requests: Default::default(),
            pending_add_peers: Default::default(),
            pending_bootstrap_requests: Default::default(),
            pending_exchange_outbound_requests: Default::default(),
            pending_commit_outbound_requests: Default::default(),
            pending_exchange_inbound_response: Default::default(),
            pending_commit_inbound_response: Default::default(),
            exchange_inbound_response_channels: Default::default(),
            commit_inbound_response_channels: Default::default(),
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

    pub fn network_event_receiver(&self) -> broadcast::Receiver<NetworkEvent> {
        self.network_event_sender.subscribe()
    }

    pub fn network_command_sender(&self) -> mpsc::Sender<Command> {
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
                // add peer for exchange protcol
                self.swarm
                    .behaviour_mut()
                    .exchange
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
            Command::DhtStartProviding { key, sender } => {
                match self.swarm.behaviour_mut().kademlia.start_providing(key) {
                    Ok(query_id) => {
                        self.pending_dht_start_providing_requests
                            .insert(query_id, sender);
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
            Command::DhtGetProviders { key, sender } => {
                let query_id = self.swarm.behaviour_mut().kademlia.get_providers(key);
                self.pending_dht_get_providers_requests
                    .insert(query_id, sender);
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
            Command::SendExchangeResponse {
                peer_id,
                request_id,
                response,
                sender,
            } => match self
                .exchange_inbound_response_channels
                .remove(&(peer_id, request_id))
            {
                Some(channel) => {
                    self.swarm
                        .behaviour_mut()
                        .exchange
                        .send_response(channel, response);
                    self.pending_exchange_inbound_response
                        .insert((peer_id, request_id), sender);
                }
                None => {
                    sender.send(Err(anyhow::anyhow!(
                        "(exchange) response channel missing for requst id {} to peer {}",
                        request_id,
                        peer_id
                    )));
                }
            },
            Command::SendExchangeRequest {
                peer_id,
                request,
                sender,
            } => {
                let request_id = self
                    .swarm
                    .behaviour_mut()
                    .exchange
                    .send_request(&peer_id, request);
                self.pending_exchange_outbound_requests
                    .insert((peer_id, request_id), sender);
            }
            Command::SendCommitResponse {
                peer_id,
                request_id,
                response,
                sender,
            } => match self
                .commit_inbound_response_channels
                .remove(&(peer_id, request_id))
            {
                Some(channel) => {
                    self.swarm
                        .behaviour_mut()
                        .commit
                        .send_response(channel, response);
                    self.pending_commit_inbound_response
                        .insert((peer_id, request_id), sender);
                }
                None => {
                    sender.send(Err(anyhow::anyhow!(
                        "(commit) response channel missing for requst id {} to peer {}",
                        request_id,
                        peer_id
                    )));
                }
            },
            Command::SendCommitRequest {
                peer_id,
                request,
                sender,
            } => {
                let request_id = self
                    .swarm
                    .behaviour_mut()
                    .commit
                    .send_request(&peer_id, request);
                self.pending_commit_outbound_requests
                    .insert((peer_id, request_id), sender);
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
            Command::SubscribeNetworkEvents { sender } => {
                sender.send(self.network_event_receiver());
            }
        }
    }

    async fn swarm_event_handler(
        &mut self,
        event: SwarmEvent<
            BehaviourEvent,
            EitherError<
                EitherError<
                    EitherError<
                        EitherError<EitherError<std::io::Error, void::Void>, ping::Failure>,
                        GossipsubHandlerError,
                    >,
                    ConnectionHandlerUpgrErr<std::io::Error>,
                >,
                ConnectionHandlerUpgrErr<std::io::Error>,
            >,
        >,
    ) {
        match event {
            SwarmEvent::Behaviour(BehaviourEvent::Mdns(event)) => {
                // emit_event(&self.network_event_sender, NetworkEvent::Mdns(event)).await;
                match event {
                    MdnsEvent::Discovered(nodes) => {
                        // Add discovered nodes to
                        // kad routing table
                        for (peer, address) in nodes {
                            debug!(
                                "Added peer with peer_id {} and address {} to kad routing table",
                                peer, address
                            );
                            self.swarm
                                .behaviour_mut()
                                .kademlia
                                .add_address(&peer, address.clone());

                            // TODO dial in here as well?
                            if let Err(e) = self.swarm.dial(address.clone()) {
                                warn!(
                                    "Dial in failed for peer_id {} at address {} with error :{}",
                                    peer, address, e
                                );
                            }
                        }
                    }
                    MdnsEvent::Expired(nodes) => {}
                }
            }
            // KademliaStoreInserts is set to FilterBoth so that we can
            // validate AddProvider request from a peer that indicates an exchange
            // with peer referenced in key using atleast one valid commit from peer
            // referenced in key.
            // Validation implementation ref - https://gitlab.com/etrovub/smartnets/glycos/glycos/-/blob/6b1f513dd5d502ca3a8d429934a87b1a2adf0640/glycos/src/net/mod.rs
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
                                sender.send(Err(anyhow::Error::from(err)));
                            }
                        }
                    }
                },
                QueryResult::StartProviding(Ok(record)) => {
                    debug!("kad: StartProviding for key {:?} success", record.key);

                    if let Some(sender) = self.pending_dht_start_providing_requests.remove(&id) {
                        sender.send(Ok(record));
                    } else {
                        emit_response_channel_missing(&id);
                    }
                }
                QueryResult::StartProviding(Err(err)) => {
                    error!(
                        "kad: StartProviding for key {:?} errored with timeout",
                        err.key()
                    );

                    if let Some(sender) = self.pending_dht_start_providing_requests.remove(&id) {
                        sender.send(Err(err.into()));
                    } else {
                        emit_response_channel_missing(&id);
                    }
                }
                QueryResult::GetProviders(Ok(record)) => {
                    debug!("kad: GetProviders for key {:?} success", record.key);

                    if let Some(sender) = self.pending_dht_get_providers_requests.remove(&id) {
                        sender.send(Ok(record));
                    } else {
                        emit_response_channel_missing(&id);
                    }
                }
                QueryResult::GetProviders(Err(err)) => {
                    error!(
                        "kad: GetProviders for key {:?} errored with timeout",
                        err.key()
                    );

                    if let Some(sender) = self.pending_dht_get_providers_requests.remove(&id) {
                        sender.send(Err(err.into()));
                    } else {
                        emit_response_channel_missing(&id);
                    }
                }
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
            SwarmEvent::Behaviour(BehaviourEvent::Exchange(event)) => {
                match event {
                    RequestResponseEvent::Message { peer, message } => {
                        match message {
                            RequestResponseMessage::Request {
                                request_id,
                                request,
                                channel,
                            } => {
                                // debug!("request_response: Received request {:?} ", request.clone());
                                self.exchange_inbound_response_channels
                                    .insert((peer, request_id), channel);
                                emit_event(
                                    &self.network_event_sender,
                                    NetworkEvent::ExchangeRequest {
                                        sender_peer_id: peer,
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
                                    .pending_exchange_outbound_requests
                                    .remove(&(peer, request_id))
                                {
                                    sender.send(Ok(response));
                                } else {
                                    error!(
                                        "(exchange) response channel missing for request id {}",
                                        request_id
                                    );
                                }
                            }
                        }
                    }
                    RequestResponseEvent::ResponseSent { peer, request_id } => {
                        if let Some(sender) = self
                            .pending_exchange_inbound_response
                            .remove(&(peer, request_id))
                        {
                            sender.send(Ok(()));
                        } else {
                            error!(
                                "(exchange) response channel missing for request id {}",
                                request_id
                            );
                        }
                    }
                    RequestResponseEvent::OutboundFailure {
                        peer,
                        request_id,
                        error,
                    } => {
                        if let Some(sender) = self
                            .pending_exchange_outbound_requests
                            .remove(&(peer, request_id))
                        {
                            sender.send(Err(error.into()));
                        } else {
                            error!(
                                "(exchange) response channel missing for request id {}",
                                request_id
                            );
                        }
                    }
                    RequestResponseEvent::InboundFailure {
                        peer,
                        request_id,
                        error,
                    } => {
                        if let Some(sender) = self
                            .pending_exchange_inbound_response
                            .remove(&(peer, request_id))
                        {
                            sender.send(Err(error.into()));
                        } else {
                            error!(
                                "(exchange) response channel missing for request id {}",
                                request_id
                            );
                        }
                    }
                };
            }
            SwarmEvent::Behaviour(BehaviourEvent::Commit(event)) => {
                match event {
                    RequestResponseEvent::Message { peer, message } => {
                        match message {
                            RequestResponseMessage::Request {
                                request_id,
                                request,
                                channel,
                            } => {
                                // debug!("request_response: Received request {:?} ", request.clone());
                                self.commit_inbound_response_channels
                                    .insert((peer, request_id), channel);
                                emit_event(
                                    &self.network_event_sender,
                                    NetworkEvent::CommitRequest {
                                        sender_peer_id: peer,
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
                                    .pending_commit_outbound_requests
                                    .remove(&(peer, request_id))
                                {
                                    sender.send(Ok(response));
                                } else {
                                    error!(
                                        "(commit) response channel missing for request id {}",
                                        request_id
                                    );
                                }
                            }
                        }
                    }
                    RequestResponseEvent::ResponseSent { peer, request_id } => {
                        if let Some(sender) = self
                            .pending_commit_inbound_response
                            .remove(&(peer, request_id))
                        {
                            sender.send(Ok(()));
                        } else {
                            error!(
                                "(commit) response channel missing for request id {}",
                                request_id
                            );
                        }
                    }
                    RequestResponseEvent::OutboundFailure {
                        peer,
                        request_id,
                        error,
                    } => {
                        if let Some(sender) = self
                            .pending_commit_outbound_requests
                            .remove(&(peer, request_id))
                        {
                            sender.send(Err(error.into()));
                        } else {
                            error!(
                                "(commit) response channel missing for request id {}",
                                request_id
                            );
                        }
                    }
                    RequestResponseEvent::InboundFailure {
                        peer,
                        request_id,
                        error,
                    } => {
                        if let Some(sender) = self
                            .pending_commit_inbound_response
                            .remove(&(peer, request_id))
                        {
                            sender.send(Err(error.into()));
                        } else {
                            error!(
                                "(commit) response channel missing for request id {}",
                                request_id
                            );
                        }
                    }
                };
            }

            SwarmEvent::IncomingConnection {
                local_addr,
                send_back_addr,
            } => {
                debug!(
                    "(swarm) incoming connection {:?} {:?} ",
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
                    "(swarm) connection established {:?} {:?} {:?} ",
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
                    "(swarm) connection closed {:?} {:?} {:?} {:?} ",
                    peer_id, endpoint, num_established, cause
                );
            }
            SwarmEvent::NewListenAddr {
                listener_id,
                address,
            } => {
                debug!(
                    "(swarm) new listener id {:?} and addr {:?} ",
                    listener_id, address
                );
                self.node_address = Some(address);
                // self.network_event_sender.send(NetworkEvent::NewListenAddr { listener_id, address}).await.expect("Network evvent message dropped");
            }
            _ => {}
        }
    }
}

async fn emit_event(sender: &broadcast::Sender<NetworkEvent>, event: NetworkEvent) {
    if sender.send(event).is_err() {
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
    DhtStartProviding {
        key: Key,
        sender: oneshot::Sender<Result<AddProviderOk, anyhow::Error>>,
    },
    DhtGet {
        key: Key,
        quorum: Quorum,
        sender: oneshot::Sender<Result<GetRecordOk, anyhow::Error>>,
    },
    DhtGetProviders {
        key: Key,
        sender: oneshot::Sender<Result<GetProvidersOk, anyhow::Error>>,
    },
    Bootstrap {
        sender: oneshot::Sender<Result<(), anyhow::Error>>,
    },
    PublishMessage {
        message: GossipsubMessage,
        sender: oneshot::Sender<Result<gossipsub::MessageId, anyhow::Error>>,
    },
    SendExchangeResponse {
        peer_id: PeerId,
        request_id: RequestId,
        response: ExchangeResponse,
        sender: oneshot::Sender<Result<(), anyhow::Error>>,
    },
    SendExchangeRequest {
        peer_id: PeerId,
        request: ExchangeRequest,
        sender: oneshot::Sender<Result<ExchangeResponse, anyhow::Error>>,
    },
    SendCommitResponse {
        peer_id: PeerId,
        request_id: RequestId,
        response: CommitResponse,
        sender: oneshot::Sender<Result<(), anyhow::Error>>,
    },
    SendCommitRequest {
        peer_id: PeerId,
        request: CommitRequest,
        sender: oneshot::Sender<Result<CommitResponse, anyhow::Error>>,
    },
    NetworkDetails {
        sender: oneshot::Sender<Result<(PeerId, Multiaddr), anyhow::Error>>,
    },
    SubscribeNetworkEvents {
        sender: oneshot::Sender<broadcast::Receiver<NetworkEvent>>,
    },
}

#[derive(Debug, Clone)]
pub enum NetworkEvent {
    GossipsubMessageRecv(GossipsubMessage),
    ExchangeRequest {
        sender_peer_id: PeerId,
        request_id: RequestId,
        request: ExchangeRequest,
    },
    CommitRequest {
        sender_peer_id: PeerId,
        request_id: RequestId,
        request: CommitRequest,
    },
    NewListenAddr {
        listener_id: ListenerId,
        address: Multiaddr,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum GossipsubMessage {
    NewQuery(storage::Query),
    Commit {
        commit: storage::Commit,
        trade_id: u32,
    },
    InvalidatingSignature {
        trade_id: u32,
        provider_wallet: Address,
        requester_wallet: Address,
        invalidating_signature: Signature,
    },
}

impl TryFrom<gossipsub::GossipsubMessage> for GossipsubMessage {
    type Error = Box<dyn std::error::Error + Send + Sync>;
    fn try_from(recv_message: gossipsub::GossipsubMessage) -> Result<Self, Self::Error> {
        match recv_message.source {
            Some(peer_id) => match serde_json::from_slice(&recv_message.data) {
                Ok(v) => Ok(v),
                Err(e) => Err(Box::new(e)),
            },
            None => Err("(gossipsub) Gossipsub message does not contain source peer id".into()),
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
    Commit,
    InvalidatingSignature,
}

impl GossipsubTopic {
    fn ident_topic(self) -> gossipsub::IdentTopic {
        match self {
            GossipsubTopic::Query => gossipsub::IdentTopic::new("Query"),
            GossipsubTopic::Commit => gossipsub::IdentTopic::new("Commit"),
            GossipsubTopic::InvalidatingSignature => {
                gossipsub::IdentTopic::new("InvalidatingSignature")
            }
        }
    }

    fn from_message(message: &GossipsubMessage) -> Self {
        match message {
            GossipsubMessage::NewQuery { .. } => GossipsubTopic::Query,
            GossipsubMessage::Commit { .. } => GossipsubTopic::Commit,
            GossipsubMessage::InvalidatingSignature { .. } => GossipsubTopic::InvalidatingSignature,
        }
    }
}
