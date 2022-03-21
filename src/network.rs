use async_std::io::prelude::BufReadExt;
use async_std::{self};
use async_std::prelude::StreamExt;
use libp2p::core::transport::Boxed;
use libp2p::futures::stream::Peek;
use libp2p::kad::record::Key;
use libp2p::kad::{Quorum, GetProvidersOk, Kademlia, KademliaEvent, QueryId, QueryResult, KademliaConfig, KadConnectionType, BootstrapOk, GetClosestPeersOk, GetClosestPeersError, BootstrapError, GetRecordOk, GetRecordError, PutRecordOk, PutRecordError, Record, Addresses};
use libp2p::kad::record::store::MemoryStore;
use libp2p::{NetworkBehaviour, PeerId, Transport, Multiaddr, Swarm};
use libp2p::swarm::{NetworkBehaviour, NetworkBehaviourEventProcess, SwarmBuilder, SwarmEvent};
use libp2p::identity::{Keypair, secp256k1};
use libp2p::dns::TokioDnsConfig;
use libp2p::tcp::TokioTcpConfig;
use libp2p::core::transport::upgrade::Version;
use libp2p::core::upgrade::SelectUpgrade;
use libp2p::core::muxing::StreamMuxerBox;
use libp2p::core::either::{EitherError};
use libp2p::core::ConnectedPoint;
use libp2p::yamux::YamuxConfig;
use libp2p::mplex::MplexConfig;
use libp2p::ping::{Ping, PingEvent, PingFailure, PingSuccess, PingConfig, self};
use libp2p::mdns::{Mdns, MdnsConfig, MdnsEvent};
use tokio::{select, io};
use tokio::sync::{mpsc, oneshot};
use libp2p::noise;
use std::error::Error;
use std::time::Duration;
use std::collections::{HashMap};


#[derive(NetworkBehaviour)]
#[behaviour(out_event = "BehaviourEvent")]
pub struct Behaviour {
    kademlia: Kademlia<MemoryStore>,
    mdns: Mdns,
    ping: Ping,
}

impl Behaviour {
    pub async fn new(peer_id: PeerId) -> Self  {
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

        Behaviour {
            kademlia,
            mdns,
            ping,
        }
    }    
}

pub enum BehaviourEvent {
    Kademlia(KademliaEvent),
    Ping(PingEvent),
    Mdns(MdnsEvent),
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
    let keypair = match seed {
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
    let behaviour = Behaviour::new(peer_id).await;
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
}

pub struct NetworkInterface {
    swarm: Swarm<Behaviour>,
    command_receiver: mpsc::Receiver<Command>,
    network_event_sender: mpsc::Sender<NetworkEvent>,
    pending_dht_put_requests: HashMap<QueryId, oneshot::Sender<Result<PutRecordOk, Box<dyn Error + Send>>>>,
    pending_dht_get_requests: HashMap<QueryId, oneshot::Sender<Result<GetRecordOk, GetRecordError>>>,
    pending_add_peers: HashMap<PeerId, oneshot::Sender<Result<(), Box<dyn Error + Send>>>>,
    pending_bootstrap_requests: HashMap<QueryId, oneshot::Sender<Result<(), Box<dyn Error + Send>>>>,
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
            known_peers: Default::default(),
        }
    }

    pub async fn run(mut self) {
        loop {
            select! {
                command = self.command_receiver.recv() => {
                    println!("Received command {:?} ", command);
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
            }
        }
    }

    async fn swarm_event_handler(&mut self, event: SwarmEvent<BehaviourEvent, EitherError<EitherError<std::io::Error, void::Void>, ping::Failure>>) {
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
                    ConnectedPoint::Listener { local_addr, send_back_addr } => {
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
            },
            _ => {return}
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
}

#[derive(Debug)]
pub enum NetworkEvent {
    Mdns(MdnsEvent),
}

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
