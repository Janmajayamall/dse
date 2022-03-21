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

    let (mut client,mut network_event_receiver,mut network_interface) = network::new(opt.seed).await.expect("network::new failed");

    tokio::spawn(network_interface.run());


    // Listen on either provided opt value or any interface
    if let Some(listen_on) = opt.listen_on {
        client.start_listening(listen_on).await;
    }else {
        client.start_listening("/ip4/0.0.0.0/tcp/0".parse()?).await;
    }

    // add bootnode
    if let (Some(boot_id),  Some(boot_addr)) = (opt.boot_id, opt.boot_addr) {
        println!("Bootnode peerId {:?} multiAddr {:?} ", boot_id, boot_addr);
        client.add_address(boot_id, boot_addr).await;
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
            event = network_event_receiver.recv() => {
                match event {
                    Some(out) => {
                       match out {
                           network::NetworkEvent::Mdns(MdnsEvent::Discovered(list)) => {
                                for node in list {
                                    println!("Discovered node with peer id {:?} and multiaddr {:?} ", node.0, node.1);
                                    client.add_address(node.0, node.1).await.expect("Client fn call dropped");
                                }
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
                    // handle_input(val, &mut swarm).await;
                }
            }
        }
    }

    Ok(())
}

// async fn handle_input(command: String, swarm: &mut Swarm<network::Behaviour>) {
//     let mut args = command.split(" ");

//     match args.next() {
//         Some("BOOTSTRAP") => {
//             match swarm.behaviour_mut().kademlia.bootstrap() {
//                 Ok(id) => {
//                     println!("Bootstrapping with query {:?} ", id);
//                 },
//                 Err(e) => {
//                     println!("No known peers for bootstrapping");
//                 }
//             }
//         },
//         Some("CLOSEST_PEERS") => {
//             if let Some(id) = args.next() {
//                 let peer_id: PeerId = id.parse().unwrap();
//                 let query_id = swarm.behaviour_mut().kademlia.get_closest_peers(peer_id);
//                 println!("Query id for closest peers to id {:?}: {:?} ", id, query_id);
//             }else {
//                 println!("Peer id not provided");
//             }
//         },
//         Some("PUT") => {
//             if let Some(key) = args.next() {
//                 let val = args.next().expect("Value needed");

//                 let record = Record {
//                     key: key.to_owned().as_bytes().to_vec().into(),
//                     value: val.as_bytes().to_vec(),
//                     publisher: None,
//                     expires: None,
//                 };
//                 let quorum = Quorum::One;

//                 match swarm.behaviour_mut().kademlia.put_record(record, quorum) {
//                     Ok(query_id) => {
//                         println!("Put query id {:?} ", query_id);
//                     },
//                     Err(e) => {
//                         println!("can't put record kad {:?} ", e);
//                     }
//                 }
//             }
//         },
//         Some("GET") => {
//             if let Some(key) = args.next() {
//                 let quorum = Quorum::One;
//                 let query_id = swarm.behaviour_mut().kademlia.get_record(key.as_bytes().to_vec().into(), quorum);
//             }
//         }
//         Some("PEERS") => {
//             let addresses = swarm.behaviour_mut().kademlia.kbuckets();
//             for a in addresses {
//                 for i in a.iter() {
//                     println!("bucket node {:?} {:?} with status {:?} ", i.node.key, i.node.value, i.status);
//                 }
//             }
//         }
//         _ => {
//             println!("Unrecognised command!");
//         }
//     }
// }
