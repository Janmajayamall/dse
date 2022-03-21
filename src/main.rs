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
mod network;

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
    let transport = network::build_transport(&keypair)?;
    let behaviour = network::Behaviour::new(peer_id).await;
    let mut swarm = SwarmBuilder::new(
        transport,
        behaviour,
        peer_id
    ).executor({
        Box::new(network::CustomExecutor)
    }).build();


    // Listen on either provided opt value or any interface
    if let Some(listen_on) = opt.listen_on {
        swarm.listen_on(listen_on)?;
    }else {
        swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;
    }


    // add bootnode
    if let (Some(boot_id),  Some(boot_addr)) = (opt.boot_id, opt.boot_addr) {
        println!("Bootnode peerId {:?} multiAddr {:?} ", boot_id, boot_addr);
        swarm.behaviour_mut().kademlia.add_address(&boot_id, boot_addr);
    }


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

async fn handle_input(command: String, swarm: &mut Swarm<network::Behaviour>) {
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
