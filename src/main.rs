use async_std::io::prelude::BufReadExt;
use async_std::{self};
use async_std::prelude::StreamExt;
use libp2p::core::transport::Boxed;
use libp2p::futures::stream::Peek;
use libp2p::kad::{GetProvidersOk, Kademlia, KademliaEvent, QueryId, QueryResult, KademliaConfig, KadConnectionType, BootstrapOk, GetClosestPeersOk, GetClosestPeersError, BootstrapError};
use libp2p::kad::record::store::MemoryStore;
use libp2p::{NetworkBehaviour, PeerId, Transport, Multiaddr, Swarm};
use libp2p::swarm::{NetworkBehaviour, NetworkBehaviourEventProcess, SwarmBuilder, SwarmEvent};
use libp2p::identity::{Keypair, secp256k1};
use libp2p::dns::TokioDnsConfig;
use libp2p::tcp::TokioTcpConfig;
use libp2p::core::transport::upgrade::Version;
use libp2p::core::upgrade::SelectUpgrade;
use libp2p::core::muxing::StreamMuxerBox;
use libp2p::yamux::YamuxConfig;
use libp2p::mplex::MplexConfig;
use libp2p::ping::{Ping, PingEvent, PingFailure, PingSuccess, PingConfig};
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
                println!("kad: outboud query completed for query id {:?} and result {:?} ", id, result);
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
                    _ => {},
                }
            }
            _ => {}
        }
    }
}

impl NetworkBehaviourEventProcess<PingEvent> for Behaviour { 
    fn inject_event(&mut self, event: PingEvent) {
        match event {
            PingEvent {
                peer,
                result: Result::Ok(
                    PingSuccess::Ping {
                        rtt
                    }
                )
            } => {
                println!(
                    "ping: rtt {:?} from {:?}", rtt, peer
                );
            },
            PingEvent {
                peer,
                result: Result::Ok(
                    PingSuccess::Pong
                )
            } => {
                println!(
                    "ping: pong from {:?}", peer
                );
            }
             PingEvent {
                peer,
                result: Result::Err(
                    PingFailure::Timeout
                )
            } => {
                println!(
                    "ping: timeout to peer {:?}", peer
                );
            }
            PingEvent {
                peer,
                result: Result::Err(
                   PingFailure::Unsupported
                )
            } => {
                println!(
                    "ping: peer {:?} does not support ping", peer
                );
            }
             PingEvent {
                peer,
                result: Result::Err(
                   PingFailure::Other {
                       error
                   }
                )
            } => {
                println!(
                    "ping: failure with peer {:?}: {:?}", peer, error
                );
            }
            
        }
    }
}


impl Behaviour {
    pub async fn new(peer_id: PeerId, boot_id: Option<PeerId>, boot_addr: Option<Multiaddr>) -> Self  {

        // setup kademlia
        let store = MemoryStore::new(peer_id);
        let mut kad_config = KademliaConfig::default();
        kad_config.set_protocol_name("kad_protocol".as_bytes());
        kad_config.set_query_timeout(Duration::from_secs(300));
        // set disjoint_query_paths to true. Ref: https://discuss.libp2p.io/t/s-kademlia-lookups-over-disjoint-paths-in-rust-libp2p/571
        kad_config.disjoint_query_paths(true);
        let mut kademlia = Kademlia::with_config(peer_id, store, kad_config);


        // ping
        let ping_config = PingConfig::new().with_keep_alive(true);
        let ping = Ping::new(ping_config);

        // add bootnode
        if let (Some(boot_id),  Some(boot_addr)) = (boot_id, boot_addr) {
            println!("Bootnode peerId {:?} multiAddr {:?} ", boot_id, boot_addr);
            kademlia.add_address(&boot_id, boot_addr);
        }

    
        Behaviour {
            kademlia,
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
    let behaviour = Behaviour::new(peer_id, opt.boot_id, opt.boot_addr).await;
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

    // bootstrap node
    match swarm.behaviour_mut().kademlia.bootstrap() {
        Ok(id) => {
            println!("Bootstrap query id {:?} ", id);
        }
        Err(e) => {
            println!("No known peers");
        }
    }

    // read std input
    let mut stdin = async_std::io::BufReader::new(async_std::io::stdin()).lines();

   
    loop {
        select! {
            event = swarm.next() => {
                println!("Swarm event {:?} ", event);
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
        _ => {
            println!("Unrecognised command!");
        }
    }
}