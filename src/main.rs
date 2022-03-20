use async_std::io::prelude::BufReadExt;
use async_std::{self};
use async_std::prelude::StreamExt;
use libp2p::core::transport::Boxed;
use libp2p::futures::stream::Peek;
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
use libp2p::noise;
use std::error;
use std::time::Duration;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(name = "DSE args")]
struct Opt {
    #[structopt(long)]
    seed: Option<u8>,

    #[structopt(long)]
    listen_on: Option<Multiaddr>,

    #[structopt(long)]
    boot_id: Option<PeerId>,

    #[structopt(long)]
    boot_addr: Option<Multiaddr>
}

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
        kad_config.set_protocol_name("kad_protocol".as_bytes());
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

// Uses TCP encrypted using noise DH and MPlex for multiplexing
pub fn build_transport(identity_keypair: &Keypair) -> io::Result<Boxed<(PeerId, StreamMuxerBox)>>{
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

struct CustomExecutor;
impl libp2p::core::Executor for CustomExecutor {
    fn exec(
        &self,
        future: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'static + Send>>,
    ) {
        tokio::task::spawn(future);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn error::Error>> {
    let opt = Opt::from_args();

    // create keypair for the node
    let keypair = match opt.seed {
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
    let mut swarm = SwarmBuilder::new(
        transport,
        behaviour,
        peer_id
    ).executor({
        Box::new(CustomExecutor)
    }).build();


    // Listen on either provided opt value or any interface
    if let Some(listen_on) = opt.listen_on {
        swarm.listen_on(listen_on)?;
    }else {
        swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;
    }


    // // add bootnode
    // if let (Some(boot_id),  Some(boot_addr)) = (opt.boot_id, opt.boot_addr) {
    //     println!("Bootnode peerId {:?} multiAddr {:?} ", boot_id, boot_addr);
    //     swarm.behaviour_mut().kademlia.add_address(&boot_id, boot_addr);
    // }


    // // bootstrap node
    // match swarm.behaviour_mut().kademlia.bootstrap() {
    //     Ok(id) => {
    //         println!("Bootstrap query id {:?} ", id);
    //     }
    //     Err(e) => {
    //         println!("No known peers");
    //     }
    // }

    // read std input
    let mut stdin = async_std::io::BufReader::new(async_std::io::stdin()).lines();

   
    loop {
        select! {
            event = swarm.next() => {
                match event {
                    Some(out) => {
                       match out {
                           SwarmEvent::IncomingConnection {local_addr, send_back_addr} => {
                            println!("swarm: incoming connection {:?} {:?} ", local_addr, send_back_addr);
                           },
                           SwarmEvent::ConnectionEstablished {peer_id, endpoint, num_established, ..} => {
                            println!("swarm: connection established {:?} {:?} {:?} ", peer_id, endpoint, num_established);
                            // match endpoint {
                            //     ConnectedPoint::Listener {local_addr, send_back_addr} => {
                            //         swarm.behaviour_mut().kademlia.add_address(&peer_id, send_back_addr);
                            //     },
                            //     _ => {}
                            // }
                           },
                           SwarmEvent::ConnectionClosed {peer_id, endpoint, num_established, cause} => {
                            println!("swarm: connection closed {:?} {:?} {:?} {:?} ", peer_id, endpoint, num_established, cause);

                           },
                           SwarmEvent::NewListenAddr {listener_id, address} => {
                            println!("swarm: new listener id {:?} and addr {:?} ", listener_id, address);
                           },
                           _ => {}
                       }
                    },  
                    None => {}
                };
            }
            line = stdin.next() => {    
                if let Some(_line) = line {
                    let val = _line.unwrap();
                    println!("line {}", val);
                    handle_input(val, &mut swarm).await;
                }
            }
        }
    }

    Ok(())
}

async fn handle_input(command: String, swarm: &mut Swarm<Behaviour>) {
    let mut args = command.split(" ");

    match args.next() {
        Some("BOOTSTRAP") => {
            match swarm.behaviour_mut().kademlia.bootstrap() {
                Ok(id) => {
                    println!("Bootstrapping with query {:?} ", id);
                },
                Err(e) => {
                    println!("No known peers for bootstrapping");
                }
            }
        },
        Some("CLOSEST_PEERS") => {
            if let Some(id) = args.next() {
                let peer_id: PeerId = id.parse().unwrap();
                let query_id = swarm.behaviour_mut().kademlia.get_closest_peers(peer_id);
                println!("Query id for closest peers to id {:?}: {:?} ", id, query_id);
            }else {
                println!("Peer id not provided");
            }
        },
        Some("PUT") => {
            if let Some(key) = args.next() {
                let val = args.next().expect("Value needed");

                let record = Record {
                    key: key.to_owned().as_bytes().to_vec().into(),
                    value: val.as_bytes().to_vec(),
                    publisher: None,
                    expires: None,
                };
                let quorum = Quorum::One;

                match swarm.behaviour_mut().kademlia.put_record(record, quorum) {
                    Ok(query_id) => {
                        println!("Put query id {:?} ", query_id);
                    },
                    Err(e) => {
                        println!("can't put record kad {:?} ", e);
                    }
                }
            }
        },
        Some("GET") => {
            if let Some(key) = args.next() {
                let quorum = Quorum::One;
                let query_id = swarm.behaviour_mut().kademlia.get_record(key.as_bytes().to_vec().into(), quorum);
            }
        }
        Some("PEERS") => {
            let addresses = swarm.behaviour_mut().kademlia.kbuckets();
            for a in addresses {
                for i in a.iter() {
                    println!("bucket node {:?} {:?} with status {:?} ", i.node.key, i.node.value, i.status);
                }
            }
        }
        _ => {
            println!("Unrecognised command!");
        }
    }
}