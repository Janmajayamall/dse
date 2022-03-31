use async_std::io::prelude::BufReadExt;
use async_std::{self};
use async_trait::async_trait;
use async_std::prelude::StreamExt;
use futures::{AsyncRead, AsyncWrite};
use libp2p::core::connection::ListenerId;
use libp2p::core::transport::Boxed;
use libp2p::kad::record::Key;
use libp2p::kad::{Quorum, Kademlia, KademliaEvent, QueryId, QueryResult, KademliaConfig, BootstrapError, GetRecordOk, GetRecordError, PutRecordOk, PutRecordError, Record, Addresses};
use libp2p::request_response::{self, RequestResponse, RequestResponseCodec, RequestResponseConfig, RequestResponseEvent, RequestResponseMessage, ProtocolSupport};
use libp2p::kad::record::store::MemoryStore;
use libp2p::{NetworkBehaviour, PeerId, Transport, Multiaddr, Swarm};
use libp2p::swarm::{ SwarmBuilder, SwarmEvent};
use libp2p::identity::{Keypair, secp256k1};
use libp2p::dns::TokioDnsConfig;
use libp2p::tcp::TokioTcpConfig;
use libp2p::core::transport::upgrade::Version;
use libp2p::core::upgrade::{SelectUpgrade, read_length_prefixed, write_length_prefixed};
use libp2p::core::muxing::StreamMuxerBox;
use libp2p::core::either::{EitherError};
use libp2p::core::ConnectedPoint;
use libp2p::yamux::YamuxConfig;
use libp2p::mplex::MplexConfig;
use libp2p::ping::{Ping, PingEvent, PingConfig, self};
use libp2p::mdns::{Mdns, MdnsConfig, MdnsEvent};
use libp2p::gossipsub::{self, Gossipsub, GossipsubEvent, error::{GossipsubHandlerError}};
use tokio::{select, io};
use tokio::sync::{mpsc, oneshot};
use libp2p::noise;
use std::error::Error;
use std::time::Duration;
use std::collections::{HashMap};
use std::convert::{TryFrom};
use serde::{Deserialize, Serialize};

use super::indexer;


#[derive(NetworkBehaviour)]
#[behaviour(out_event = "BehaviourEvent")]
pub struct Behaviour {
    kademlia: Kademlia<MemoryStore>,
    mdns: Mdns,
    ping: Ping,
    gossipsub: Gossipsub,
    request_response: RequestResponse<DseMessageCodec>
}

impl Behaviour {
    pub async fn new(keypair: &Keypair) -> Self  {
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
        let gossipsub_config = gossipsub::GossipsubConfigBuilder::default().validation_mode(gossipsub::ValidationMode::Strict).build().expect("Gossipsub config build failed!");
        let mut gossipsub = Gossipsub::new(gossipsub::MessageAuthenticity::Signed(keypair.clone()), gossipsub_config).expect("Gossipsub failed to initialise!");

        // by default subcribe to search query topic
        gossipsub.subscribe(&GossipsubTopic::Query.ident_topic()).expect("Gossipsub subscription to topic SearchQuery failed!");

        // request response
        let request_response = RequestResponse::new(DseMessageCodec(), std::iter::once((DseMessageProtocol(), ProtocolSupport::Full)), Default::default());
        
        Behaviour {
            kademlia,
            mdns,
            ping,
            gossipsub,
            request_response,
        }
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

/// Creates the client + network interface. Also
/// sets up network event stream
pub async fn new(
    seed: Option<u8>
) -> Result<(Client, mpsc::Receiver<NetworkEvent>, NetworkInterface), Box<dyn Error>> {
    let keypair =    match seed {
        Some(seed) => {
            let mut bytes = [0u8; 32];
            bytes[0] = seed;
            let secret_key = secp256k1::SecretKey::from_bytes(bytes).expect("Invalid seed");
            Keypair::Secp256k1(secret_key.into())
        },
        None => Keypair::generate_secp256k1(),
    };
    let peer_id = keypair.public().to_peer_id();
    println!("Node peer id {:?} ", peer_id.to_base58());

    // Build swarm
    let transport = build_transport(&keypair)?;
    let behaviour = Behaviour::new(&keypair).await;
    let swarm = SwarmBuilder::new(
        transport,
        behaviour,
        peer_id
    ).executor({
        Box::new(CustomExecutor)
    }).build();

    let (command_sender, command_receiver) = mpsc::channel::<Command>(10);
    let (network_event_sender, network_event_receiver) = mpsc::channel::<NetworkEvent>(10);

    // network interface
    let network_interface = NetworkInterface::new(swarm, command_receiver, network_event_sender);
    let client = Client {
        command_sender,
    };

    Ok((
        client,
        network_event_receiver,
        network_interface,
    ))
}   

/// Uses TCP encrypted using noise DH and MPlex for multiplexing
pub fn build_transport(identity_keypair: &Keypair) -> io::Result<Boxed<(PeerId, StreamMuxerBox)>> {
    // noise config
    let keypair = noise::Keypair::<noise::X25519>::new().into_authentic(identity_keypair).unwrap();
    let noise_config = noise::NoiseConfig::xx(keypair).into_authenticated();

    Ok( TokioDnsConfig::system(TokioTcpConfig::new())?
    .upgrade(Version::V1)
    .authenticate(noise_config)
    .multiplex(
        SelectUpgrade::new(
            YamuxConfig::default(),
            MplexConfig::new()
        )
    )
    .timeout(Duration::from_secs(20))
    .map(|(peer_id, muxer), _| (peer_id, StreamMuxerBox::new(muxer)))
    .map_err(|err| io::Error::new(io::ErrorKind::Other, err))
    .boxed())
}

// client & event loop for network
#[derive(Clone)]
pub struct Client {
    command_sender: mpsc::Sender<Command>,
}

impl Client {
    pub async fn add_address(&mut self, peer_id: PeerId, peer_addr: Multiaddr) ->  Result<(), Box<dyn Error + Send>>{
        let (sender, receiver) = oneshot::channel();
        self.command_sender.send(
            Command::AddPeer { peer_id, peer_addr, sender }   
        ).await.expect("Command message dropped");
        receiver.await.expect("Client response message dropped")
    }

    pub async fn start_listening(&mut self, addr: Multiaddr) -> Result<(), Box<dyn Error + Send>> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender.send(
            Command::StartListening {
                addr,
                sender
            }
        ).await.expect("Command message dropped");
        receiver.await.expect("Client response message dropped")
    }

    pub async fn dht_put(&mut self, record: Record, quorum: Quorum) -> Result<PutRecordOk, Box<dyn Error + Send>> {
        let (sender, receiver) = oneshot::channel();
         self.command_sender.send(
            Command::DhtPut { record, quorum, sender } 
        ).await.expect("Command message dropped");
        receiver.await.expect("Client response message dropped")
    }

    pub async fn dht_get(&mut self, key: Key, quorum: Quorum) -> Result<GetRecordOk, GetRecordError> {
        let (sender, receiver) = oneshot::channel();
         self.command_sender.send(
            Command::DhtGet { key, quorum, sender } 
        ).await.expect("Command message dropped");
        receiver.await.expect("Client response message dropped")
    }

    pub async fn kad_bootstrap(&mut self) -> Result<(), Box<dyn Error + Send>> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender.send(
            Command::Bootstrap { sender }
        ).await.expect("Command message dropped");
        receiver.await.expect("Client response message dropped")
    }

    pub async fn publish_message(&mut self, message: GossipsubMessage) -> Result<gossipsub::MessageId, Box<dyn Error + Send>> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender.send(
            Command::PublishMessage { message, sender }
        ).await.expect("Command message dropped");
        receiver.await.expect("Client response message dropped")
    }

    pub async fn send_dse_message_request(&mut self, peer_id: PeerId, message: DseMessageRequest) -> Result<DseMessageResponse, Box<dyn Error + Send>> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender.send(
            Command::SendDseMessageRequest { peer_id, message, sender }
        ).await.expect("Command message dropped");
        receiver.await.expect("Client response message dropped")
    }

    pub async fn send_dse_message_response(&mut self, request_id: request_response::RequestId, response: DseMessageResponse ) -> Result<(), Box<dyn Error + Send>> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender.send(
            Command::SendDseMessageResponse { request_id, response, sender }
        ).await.expect("Command message dropped");
        receiver.await.expect("Client response message dropped")
    }
}


pub struct NetworkInterface {
    swarm: Swarm<Behaviour>,
    command_receiver: mpsc::Receiver<Command>,
    network_event_sender: mpsc::Sender<NetworkEvent>,
    pending_dht_put_requests: HashMap<QueryId, oneshot::Sender<Result<PutRecordOk, Box<dyn Error + Send>>>>,
    pending_dht_get_requests: HashMap<QueryId, oneshot::Sender<Result<GetRecordOk, GetRecordError>>>,
    pending_add_peers: HashMap<PeerId, oneshot::Sender<Result<(), Box<dyn Error + Send>>>>,
    pending_bootstrap_requests: HashMap<QueryId, oneshot::Sender<Result<(), Box<dyn Error + Send>>>>,
    // Pending result of outbound request. It returns DseMessageResponse
    // which means the fn does not finishes until peer to which outcound request
    // is sent, responds using channel or channel drops.
    pending_dse_outbound_message_requests: HashMap<request_response::RequestId, oneshot::Sender<Result<DseMessageResponse, Box<dyn Error + Send>>>>,
    // Pending result of response sent to an inbound request
    pending_dse_inbound_message_response: HashMap<request_response::RequestId, oneshot::Sender<Result<(), Box<dyn Error + Send>>>>,
    // Stores response channels of an inbound request
    response_channels_for_inbound_requests: HashMap<request_response::RequestId, request_response::ResponseChannel<DseMessageResponse>>,
    known_peers: HashMap<PeerId, Addresses>,
}

impl NetworkInterface {
    pub fn new(
        swarm: Swarm<Behaviour>,
        command_receiver: mpsc::Receiver<Command>,
        network_event_sender: mpsc::Sender<NetworkEvent>,
    ) -> Self {
        Self {
            swarm, 
            command_receiver,
            network_event_sender,
            pending_dht_put_requests: Default::default(),
            pending_dht_get_requests: Default::default(),
            pending_add_peers: Default::default(),
            pending_bootstrap_requests: Default::default(),
            pending_dse_outbound_message_requests: Default::default(),
            pending_dse_inbound_message_response: Default::default(),
            response_channels_for_inbound_requests: Default::default(),
            known_peers: Default::default(),
        }
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
                            self.swarm_event_handler(event).await
                        }
                        None => {return}
                    }
                }
            }
        }
    }

    async fn command_handler(&mut self, command: Command){
        match command {
            Command::StartListening { addr, sender } => {
                match self.swarm.listen_on(addr) {
                    Ok(_) => {
                        let _ = sender.send(Ok(()));
                    },
                    Err(e) => {
                        let _ = sender.send(Err(Box::new(e)));
                    },
                };
            },
            Command::AddPeer { peer_id, peer_addr, sender} => {
                self.swarm.behaviour_mut().kademlia.add_address(&peer_id, peer_addr);
                let _ = sender.send(Ok(()));
            },
            Command::DhtPut { record, quorum, sender } => {
                match self.swarm.behaviour_mut().kademlia.put_record(record.clone(), quorum) {
                    Ok(query_id) => {
                        self.pending_dht_put_requests.insert(query_id, sender);
                    },
                    Err(e) => {
                        let _ = sender.send(Err(Box::new(e)));
                    }
                }
            },
            Command::DhtGet { key, quorum, sender } => {
                let query_id = self.swarm.behaviour_mut().kademlia.get_record(key, quorum);
                self.pending_dht_get_requests.insert(query_id, sender);
            },
            Command::Bootstrap{sender} => {
                match self.swarm.behaviour_mut().kademlia.bootstrap() {
                    Ok(query_id) => {
                        self.pending_bootstrap_requests.insert(query_id, sender);
                    },
                    Err(e) => {
                        let _ = sender.send(Err(Box::new(e)));
                    }
                }
            },
            Command::PublishMessage { message, sender} => {
                match self.swarm.behaviour_mut().gossipsub.publish(message.topic(), message) {
                    Ok(message_id) => {
                        let _ = sender.send(Ok(message_id));
                    },
                    Err(e) => {
                        let _ = sender.send(Err(Box::new(e)));
                    }
                }
            },
            Command::SendDseMessageResponse { request_id, response, sender } => {  
                match self.response_channels_for_inbound_requests.remove(&request_id) { 
                    Some(channel)=>{
                          match self.swarm.behaviour_mut().request_response.send_response(channel, response) {
                            Ok(_) => {
                                self.pending_dse_inbound_message_response.insert(request_id, sender);
                            }
                            Err(_) => {
                                // send_response returns Error, when
                                // ResponseChannel is already closed
                                // due to Timeout or Connection Err.
                                // Thus, we send InboundFailure::Timeout 
                                // here as the error.
                                let _ = sender.send(Err(Box::new(request_response::InboundFailure::Timeout)));
                            }
                        }
                    },
                    None => {
                        // FIX: InboundFailure isn't necessary here.
                        // Replace it with something like 
                        // "Channel not found".
                        let _ = sender.send(Err(Box::new(request_response::InboundFailure::Timeout)));
                    }
                }
              
            }
            Command::SendDseMessageRequest { peer_id, message, sender } => {
                let request_id = self.swarm.behaviour_mut().request_response.send_request(&peer_id, message);
                self.pending_dse_outbound_message_requests.insert(request_id, sender);
            }
        }
    }

    async fn swarm_event_handler(&mut self, event: SwarmEvent<BehaviourEvent, EitherError<EitherError<EitherError<std::io::Error, void::Void>, ping::Failure>, GossipsubHandlerError>>) {
        match event {
            SwarmEvent::Behaviour(BehaviourEvent::Mdns(
                event
            )) => {
                self.network_event_sender.send(NetworkEvent::Mdns(event)).await.expect("Network event message dropped");
            },
            SwarmEvent::Behaviour(BehaviourEvent::Kademlia(
                KademliaEvent::OutboundQueryCompleted{id, result, ..} 
            )) => { 
                match result { 
                    QueryResult::PutRecord(Ok(record)) => {
                        let _ = self.pending_dht_put_requests.remove(&id).expect("Put Request should be pending!").send(Ok(record));
                    },
                    QueryResult::PutRecord(Err(err)) => {
                        let _ = self.pending_dht_put_requests.remove(&id).expect("Put Request should be pending!").send(Err(Box::new(err)));
                    },
                    QueryResult::GetRecord(Ok(record)) => {
                        let _ = self.pending_dht_get_requests.remove(&id).expect("Get Request should be pending!").send(Ok(record));
                    },
                    QueryResult::GetRecord(Err(err)) => {
                        let _ = self.pending_dht_get_requests.remove(&id).expect("Get Request should be pending!").send(Err(err));
                    },
                    QueryResult::Bootstrap(Ok(record)) => {
                        println!("kad: Bootstrap with peer id {:?} success; num remaining {:?} ", record.peer, record.num_remaining);
                        if record.num_remaining == 0 {
                            let _ = self.pending_bootstrap_requests.remove(&id).expect("Bootstrap request should be pending!").send(Ok(()));
                        }
                    },
                    QueryResult::Bootstrap(Err(err)) => {
                        match err {
                            BootstrapError::Timeout {
                                peer, num_remaining
                            } => {
                                eprintln!("kad: Bootstrap with peer id {:?} failed; num remaining {:?} ", peer, num_remaining);
                                if num_remaining.is_none() || num_remaining.unwrap() == 0 {
                                    let _ = self.pending_bootstrap_requests.remove(&id).expect("Bootstrap request should be pending!").send(Err(Box::new(err)));
                                }
                            }
                        }
                    },
                    _ => {return}
                }
            },
            SwarmEvent::Behaviour(BehaviourEvent::Kademlia(
                KademliaEvent::RoutingUpdated { peer, addresses, .. }
            )) => { 
                println!("kad: routing updated with peerId {:?} address {:?} ", peer, addresses);

                // TODO: I haven't taken care of eviction of old peers here
                self.known_peers.insert(peer, addresses);
            },
            SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(
                GossipsubEvent::Message {message , ..} 
            )) => {
                println!("gossipsub: Received message {:?} ", message.clone());
                match GossipsubMessage::try_from(message) {
                    Ok(m) => {self.network_event_sender.send(NetworkEvent::GossipsubMessageRecv(m)).await.expect("Network event message dropped");},
                    Err(e) => {self.network_event_sender.send(NetworkEvent::GossipsubMessageRecvErr(e)).await.expect("Network event message dropped");}
                }
            },
            SwarmEvent::Behaviour(BehaviourEvent::RequestResponse(
                RequestResponseEvent::Message { peer, message }
            )) => {
                match message {
                    RequestResponseMessage::Request { request_id, request, channel} => {
                       self.response_channels_for_inbound_requests.insert(request_id.clone(), channel);
                       self.network_event_sender.send(NetworkEvent::DseMessageRequestRecv { peer_id: peer, request_id, request }).await.expect("Network evvent message dropped");
                    },
                    RequestResponseMessage::Response { request_id, response } => {
                        self.pending_dse_outbound_message_requests.remove(&request_id).expect("Outbound Request should be pending!").send(Ok(response));
                    }
                }
            },
            SwarmEvent::Behaviour(BehaviourEvent::RequestResponse(
                RequestResponseEvent::ResponseSent { peer, request_id }
            )) => {
                self.pending_dse_inbound_message_response.remove(&request_id).expect("Outbound Request should be pending!").send(Ok(()));
            },
            SwarmEvent::Behaviour(BehaviourEvent::RequestResponse(
                RequestResponseEvent::OutboundFailure { peer, request_id, error } 
            )) => {
                self.pending_dse_outbound_message_requests.remove(&request_id).expect("Outbound Request should be pending!").send(Err(Box::new(error)));
            },
            SwarmEvent::Behaviour(BehaviourEvent::RequestResponse(
                RequestResponseEvent::InboundFailure { peer, request_id, error } 
            )) => {
                // RequestResponseEvent::InboundFailure is thrown anytime
                // (like channel timeout of channel dropped) irrespective 
                // of whether there was a response initiated by client commands.
                match self.pending_dse_inbound_message_response.remove(&request_id) {
                    Some(sender) => {sender.send(Err(Box::new(error)));}
                    None => {}
                }
            },
            SwarmEvent::IncomingConnection {local_addr, send_back_addr} => {
                println!("swarm: incoming connection {:?} {:?} ", local_addr, send_back_addr);
            },
            SwarmEvent::ConnectionEstablished {peer_id, endpoint, num_established, ..} => {
                println!("swarm: connection established {:?} {:?} {:?} ", peer_id, endpoint, num_established);
                match endpoint {
                    // Not sure whether this is the right practice for populating 
                    // kad dht routing table of a node. 
                    // At present, this is only needed for a bootstrap node since it needs
                    // to maintain an uptodate view of kad routing table to facilitate bootstrapping
                    // of new nodes to the network. 
                    // We can skip this by a separate request that new node sends to be added
                    // to bootstrap node's routing table. 
                    ConnectedPoint::Listener { send_back_addr , ..} => {
                        if !self.known_peers.contains_key(&peer_id) {
                            self.swarm.behaviour_mut().kademlia.add_address(&peer_id, send_back_addr.clone());
                        }
                    }
                    _ => {}
                }
            },
            SwarmEvent::ConnectionClosed {peer_id, endpoint, num_established, cause} => {
                println!("swarm: connection closed {:?} {:?} {:?} {:?} ", peer_id, endpoint, num_established, cause);
            },
            SwarmEvent::NewListenAddr {listener_id, address} => {
                println!("swarm: new listener id {:?} and addr {:?} ", listener_id, address);
                self.network_event_sender.send(NetworkEvent::NewListenAddr { listener_id, address}).await.expect("Network evvent message dropped");
            },
            _ => {}
        }
    }

}

#[derive(Debug)]
pub enum Command {
    StartListening {
        addr: Multiaddr,
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    },
    AddPeer {
        peer_id: PeerId,
        peer_addr: Multiaddr,
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    },
    DhtPut {
        record: Record,
        quorum: Quorum,
        sender: oneshot::Sender<Result<PutRecordOk, Box<dyn Error + Send>>>,
    },
    DhtGet {
        key: Key,
        quorum: Quorum,
        sender: oneshot::Sender<Result<GetRecordOk, GetRecordError>>,
    },
    Bootstrap {
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    },
    PublishMessage {
        message: GossipsubMessage,
        sender: oneshot::Sender<Result<gossipsub::MessageId, Box<dyn Error + Send>>>,
    },
    SendDseMessageResponse {
        request_id: request_response::RequestId,
        response: DseMessageResponse,
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    },
    SendDseMessageRequest {
        peer_id: PeerId,
        message: DseMessageRequest,
        sender: oneshot::Sender<Result<DseMessageResponse, Box<dyn Error + Send>>>,
    }
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
    }
}

#[derive(Debug)]
#[derive(Serialize, Deserialize)]
pub enum GossipsubMessage {
    NewQuery(indexer::QueryReceived),
}

impl TryFrom<gossipsub::GossipsubMessage> for GossipsubMessage {
    type Error = Box<dyn std::error::Error + Send + Sync>;
    fn try_from(recv_message: gossipsub::GossipsubMessage) -> Result<Self, Self::Error> {
        match recv_message.source {
            Some(peer_id) => {
                match serde_json::from_slice(&recv_message.data) {
                    Ok(v) => Ok(v),
                    Err(e) => Err(Box::new(e))
                }
            },
            None => {
                Err("gossipsub: Gossipsub message does not contain source peer id".into())
            }
        }
    }
}

impl Into<Vec<u8>> for GossipsubMessage {
    fn into(self) -> Vec<u8> {
        match serde_json::to_vec(&self) {
            Ok(v) => v,
            Err(_) => Default::default()
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
            GossipsubTopic::Query => {
                gossipsub::IdentTopic::new("Query")
            },
        }
    }

    fn from_message(message: &GossipsubMessage) -> Self {
        match message {
            GossipsubMessage::NewQuery {..} => {
                GossipsubTopic::Query
            }
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub enum CommitRequest {
    // Ask for commitment 
    CommitFund {
        query_id: indexer::QueryId,
        round: u32,
    },
    // Invalidating Signature
    // Sent by service requester
    InvalidatingSignature {
        query_id: indexer::QueryId,
        invalidating_signature: String,
    },
    // End commit procedure
    EndCommit(indexer::QueryId),
}

#[derive(Deserialize, Serialize, Debug)]
pub enum CommitResponse {
    // CommitFund response
    CommitFund  {
        query_id: indexer::QueryId,
        round: u32,
        commitment: commitment::Commit,
    },
    // Ack InvalidatingSignature
    AckInvalidatingSignature(indexer::QueryId),
    // Ack End Commit
    AckEndCommit(indexer::QueryId),
}

// All stuff related to request response
#[derive(Deserialize, Serialize, Debug)]
pub enum DseMessageRequest {
    // Place bid for a query
    PlaceBid(indexer::BidReceived),
    // Accept a bid for a query
    AcceptBid(indexer::QueryId),
    // Start commit procedure
    StartCommit(indexer::QueryId),
    // Ask for wallet address
    WalletAddress(indexer::QueryId),
    // Requests related to commit fund
    Commit(CommitRequest)
}

#[derive(Deserialize, Serialize, Debug)]
pub enum DseMessageResponse {
    // Ack that bid was received
    AckBid(indexer::QueryId),
    // Ack that bid acceptance was received
    AckAcceptBid(indexer::QueryId),
    // Ack that start commit was received
    AckStartCommit(indexer::QueryId),
    // Wallet address response
    WalletAddress {
       query_id: indexer::QueryId,
       wallet_address: ethers::types::Address, 
    },
    // Respons related to commit fund 
    Commit(CommitResponse),
}

#[derive(Debug, Clone)]
pub struct DseMessageProtocol ();

impl request_response::ProtocolName for DseMessageProtocol { 
    fn protocol_name(&self) -> &[u8] {
        "/dse-message/1".as_bytes()
    }
}

#[derive(Clone)]
struct DseMessageCodec ();


#[async_trait]
impl RequestResponseCodec for DseMessageCodec {
    type Protocol = DseMessageProtocol;
    type Request = DseMessageRequest;
    type Response = DseMessageResponse;

    async fn read_request<T>(
        &mut self, 
        protocol: &Self::Protocol, 
        io: &mut T
    ) 
    -> io::Result<Self::Request> 
    where T: AsyncRead + Unpin + Send
    {
        let vec = read_length_prefixed(io, 10000000).await?;
        match serde_json::from_slice(&vec) {
            Ok(v) => Ok(v),
            Err(e) => Err(io::ErrorKind::Other.into())
        }
    }
   
    async fn read_response<T>(
        &mut self, 
        protocol: &Self::Protocol, 
        io: &mut T
    ) -> io::Result<Self::Response> 
    where T: AsyncRead + Unpin + Send
    {
        let vec = read_length_prefixed(io, 10000000).await?;
        match serde_json::from_slice(&vec) {
            Ok(v) => Ok(v),
            Err(_) => Err(io::ErrorKind::Other.into())
        }
    }

    async fn write_request<T>(
        &mut self, 
        protocol: &Self::Protocol, 
        io: &mut T, 
        req: Self::Request
    ) -> io::Result<()> 
    where T: AsyncWrite + Unpin + Send
    {
        match serde_json::to_vec(&req) {
            Ok(val) => {
                write_length_prefixed(io, &val).await?;
                Ok(())
            },
            Err(_) => Err(io::ErrorKind::Other.into())
        }
    }

    async fn write_response<T>(
        &mut self, 
        protocol: &Self::Protocol, 
        io: &mut T, 
        res: Self::Response
    ) -> io::Result<()> 
    where T: AsyncWrite + Unpin + Send
    {
        match serde_json::to_vec(&res) {
            Ok(val) => {
                write_length_prefixed(io, &val).await?;
                Ok(())
            },
            Err(_) => Err(io::ErrorKind::Other.into())
        }
    }
}


// #[derive(Debug)]

// impl NetworkBehaviourEventProcess<PingEvent> for Behaviour { 
//     fn inject_event(&mut self, event: PingEvent) {
//         // match event {
//         //     PingEvent {
//         //         peer,
//         //         result: Result::Ok(
//         //             PingSuccess::Ping {
//         //                 rtt
//         //             }
//         //         )
//         //     } => {
//         //         println!(
//         //             "ping: rtt {:?} from {:?}", rtt, peer
//         //         );
//         //     },
//         //     PingEvent {
//         //         peer,
//         //         result: Result::Ok(
//         //             PingSuccess::Pong
//         //         )
//         //     } => {
//         //         println!(
//         //             "ping: pong from {:?}", peer
//         //         );
//         //     }
//         //      PingEvent {
//         //         peer,
//         //         result: Result::Err(
//         //             PingFailure::Timeout
//         //         )
//         //     } => {
//         //         println!(
//         //             "ping: timeout to peer {:?}", peer
//         //         );
//         //     }
//         //     PingEvent {
//         //         peer,
//         //         result: Result::Err(
//         //            PingFailure::Unsupported
//         //         )
//         //     } => {
//         //         println!(
//         //             "ping: peer {:?} does not support ping", peer
//         //         );
//         //     }
//         //      PingEvent {
//         //         peer,
//         //         result: Result::Err(
//         //            PingFailure::Other {
//         //                error
//         //            }
//         //         )
//         //     } => {
//         //         println!(
//         //             "ping: failure with peer {:?}: {:?}", peer, error
//         //         );
//         //     }
            
//         // }
   
//     }
// }
