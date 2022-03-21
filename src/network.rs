use async_std::io::prelude::BufReadExt;
use async_std::{self};
use async_std::prelude::StreamExt;
use libp2p::core::transport::Boxed;
use libp2p::futures::stream::Peek;
use libp2p::kad::record::Key;
use libp2p::kad::{Quorum, GetProvidersOk, Kademlia, KademliaEvent, QueryId, QueryResult, KademliaConfig, KadConnectionType, BootstrapOk, GetClosestPeersOk, GetClosestPeersError, BootstrapError, GetRecordOk, GetRecordError, PutRecordOk, PutRecordError, Record};
use libp2p::kad::record::store::MemoryStore;
use libp2p::{NetworkBehaviour, PeerId, Transport, Multiaddr, Swarm};
use libp2p::swarm::{NetworkBehaviour, NetworkBehaviourEventProcess, SwarmBuilder, SwarmEvent};
use libp2p::identity::{Keypair, secp256k1};
use libp2p::dns::TokioDnsConfig;
use libp2p::tcp::TokioTcpConfig;
use libp2p::core::transport::upgrade::Version;
use libp2p::core::upgrade::SelectUpgrade;
use libp2p::core::muxing::StreamMuxerBox;
use libp2p::core::ConnectedPoint;
use libp2p::yamux::YamuxConfig;
use libp2p::mplex::MplexConfig;
use libp2p::ping::{Ping, PingEvent, PingFailure, PingSuccess, PingConfig};
use libp2p::mdns::{Mdns, MdnsConfig, MdnsEvent};
use tokio::io::AsyncBufReadExt;
use tokio::{select, io};
use tokio::sync::{mpsc, oneshot};
use libp2p::noise;
use std::error::Error;
use std::time::Duration;
use structopt::StructOpt;

#[derive(NetworkBehaviour)]
#[behaviour(event_process = true)]
pub struct Behaviour {
    kademlia: Kademlia<MemoryStore>,
    mdns: Mdns,
    ping: Ping,
}

impl NetworkBehaviourEventProcess<KademliaEvent> for Behaviour {
    fn inject_event(&mut self, event: KademliaEvent) {
        match event {
            KademliaEvent::RoutingUpdated { peer, is_new_peer ,addresses, bucket_range, old_peer } => {
                println!("kad: routing updated with peerId {:?} address {:?} ", peer, addresses);
            }
            KademliaEvent::InboundRequest{ request } => {
                println!("kad: received inbound request {:?} ", request);
            }
            KademliaEvent::OutboundQueryCompleted { id, result, stats } => {
                match result {
                    QueryResult::Bootstrap(Ok(BootstrapOk { peer, num_remaining })) => {
                        println!("kad: bootstrapped with peer: {:?}; remaining peer count {} ", peer, num_remaining);
                    },
                    QueryResult::Bootstrap(Err(BootstrapError::Timeout{peer, num_remaining})) => {
                        println!("kad: timedout while bootstrapping");
                    }
                    QueryResult::GetClosestPeers(Ok(GetClosestPeersOk{key, peers})) => {
                        println!("kad: closest peers with key {:?}: {:?}", key, peers);
                    },
                    QueryResult::GetClosestPeers(Err(GetClosestPeersError::Timeout{key, peers})) => {
                        println!("kad: timedout while getting closest peer to key {:?} ", key);
                    },
                    QueryResult::GetRecord(Ok(GetRecordOk {records, cache_candidates})) => {
                        for r in records.into_iter() {
                            println!("kad: got peer record for key {:?} with value {:?} ", r.record.key, r.record.value);
                        }
                    }
                    QueryResult::GetRecord(Err(GetRecordError::Timeout {key, records, quorum})) => {
                        println!("kad: timed out while getting record with key {:?} ", key);
                    },
                    QueryResult::GetRecord(Err(GetRecordError::QuorumFailed { key, records, quorum })) => {
                        println!("kad: quorum failed for getting record with key {:?} ", key);
                    },
                    QueryResult::GetRecord(Err(GetRecordError::NotFound { key, closest_peers })) => {
                        println!("kad: record with key {:?} not found", key);
                    },
                    QueryResult::PutRecord(Ok(PutRecordOk { key })) => {
                        println!("kad: put record for key {:?} was successful ", key);
                    }
                    QueryResult::PutRecord(Err(PutRecordError::Timeout {key, success, quorum})) => {
                        println!("kad: timed out while putting record with key {:?} ", key);
                    },
                    QueryResult::PutRecord(Err(PutRecordError::QuorumFailed { key, success, quorum })) => {
                        println!("kad: quorum failed for putting record with key {:?} success {:?} ", key, success);
                    },
                    _ => {},
                }
            }
            _ => {}
        }
    }
}

impl NetworkBehaviourEventProcess<MdnsEvent> for Behaviour {
    fn inject_event(&mut self, event: MdnsEvent){
        match event {
            MdnsEvent::Discovered(list) => {
                for peer in list {
                    println!("mdns: Discoverd and adding peer {:?} {:?} ", peer.0, peer.1);
                    self.kademlia.add_address(&peer.0, peer.1);
                }
            },
            _ => {}
        }
    }
}

impl NetworkBehaviourEventProcess<PingEvent> for Behaviour { 
    fn inject_event(&mut self, event: PingEvent) {
        // match event {
        //     PingEvent {
        //         peer,
        //         result: Result::Ok(
        //             PingSuccess::Ping {
        //                 rtt
        //             }
        //         )
        //     } => {
        //         println!(
        //             "ping: rtt {:?} from {:?}", rtt, peer
        //         );
        //     },
        //     PingEvent {
        //         peer,
        //         result: Result::Ok(
        //             PingSuccess::Pong
        //         )
        //     } => {
        //         println!(
        //             "ping: pong from {:?}", peer
        //         );
        //     }
        //      PingEvent {
        //         peer,
        //         result: Result::Err(
        //             PingFailure::Timeout
        //         )
        //     } => {
        //         println!(
        //             "ping: timeout to peer {:?}", peer
        //         );
        //     }
        //     PingEvent {
        //         peer,
        //         result: Result::Err(
        //            PingFailure::Unsupported
        //         )
        //     } => {
        //         println!(
        //             "ping: peer {:?} does not support ping", peer
        //         );
        //     }
        //      PingEvent {
        //         peer,
        //         result: Result::Err(
        //            PingFailure::Other {
        //                error
        //            }
        //         )
        //     } => {
        //         println!(
        //             "ping: failure with peer {:?}: {:?}", peer, error
        //         );
        //     }
            
        // }
   
    }
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

pub struct CustomExecutor;
impl libp2p::core::Executor for CustomExecutor {
    fn exec(
        &self,
        future: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'static + Send>>,
    ) {
        tokio::task::spawn(future);
    }
}

// Uses TCP encrypted using noise DH and MPlex for multiplexing
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
}

impl Client {
    pub fn add_address(&mut self, peer_id: PeerId, peer_addr: Multiaddr) {
        
    }

}


pub struct NetworkInterface {
    swarm: Swarm<Behaviour>,
    command_receiver: mpsc::Receiver<Command>,
    network_event_sender: mpsc::Sender<NetworkEvent>,
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
            network_event_sender
        }
    }

    pub async fn run(mut self) {
        loop {
            select! {
                command = self.command_receiver.recv() => {
                    match command {
                        Some(val) => {
                            // handle the command
                        },
                        None => {return},
                    }
                }
                swarm_event = self.swarm.next() => {
                    match swarm_event {
                        Some(event) => {
                            // handle swarm events
                        }
                        None => {return}
                    }
                }
            }
        }
    }

    pub async fn command_handler(command: Command){

    }


}

enum Command {
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
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    },
    DhtGet {
        key: Key,
        sender: oneshot::Sender<Result<(), Box<dyn Error + Send>>>,
    }
}


enum NetworkEvent {

}