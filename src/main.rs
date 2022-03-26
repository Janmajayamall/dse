use async_std::io::prelude::BufReadExt;
use async_std::{self};
use async_std::prelude::StreamExt;
use libp2p::kad::record::Key;
use libp2p::kad::{Quorum, Record};
use libp2p::{PeerId, Multiaddr};
use libp2p::mdns::{ MdnsEvent};
use libp2p::gossipsub::{self};
use tokio::{select};
use std::error;
use structopt::StructOpt;
mod network;
mod handler;
mod indexer;

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

    let (mut network_client,mut network_event_receiver, network_interface) = network::new(opt.seed).await.expect("network::new failed");
    let (indexer_client, indexer_event_receiver, indexer_interface) = indexer::new();

    tokio::spawn(network_interface.run());
    tokio::spawn(indexer_interface.run());


    // Listen on either provided opt value or any interface
    if let Some(listen_on) = opt.listen_on {
        let _ = network_client.start_listening(listen_on).await;
    }else {
        let _ = network_client.start_listening("/ip4/0.0.0.0/tcp/0".parse()?).await;
    }

    // add bootnode
    if let (Some(boot_id),  Some(boot_addr)) = (opt.boot_id, opt.boot_addr) {
        println!("Bootnode peerId {:?} multiAddr {:?} ", boot_id, boot_addr);
        let _ = network_client.add_address(boot_id, boot_addr).await;
    }

    // read std input
    let mut stdin = async_std::io::BufReader::new(async_std::io::stdin()).lines();

    loop {
        select! {
            event = network_event_receiver.recv() => {
                match event {
                    Some(out) => {
                        // println!("NetworkEvent: received {:?} ", out);
                        match out {
                           network::NetworkEvent::Mdns(MdnsEvent::Discovered(list)) => {
                                for node in list {
                                    println!("Discovered node with peer id {:?} and multiaddr {:?} ", node.0, node.1);
                                    client.add_address(node.0, node.1).await.expect("Client fn call dropped");
                                }
                           },
                           network::NetworkEvent::GossipsubMessageRecv(message) => {
                                match message {
                                    network::GossipsubMessage::SearchQuery(query) => {
                                        
                                    },
                                    _ => {}
                                }
                           },
                           network::NetworkEvent::DseMessageRequestRecv {request_id, request, channel} => {
                                match request {
                                    network::DseMessageRequest::PlaceBid {query_id, bid} => {
                                        // ACK the bid
                                    },
                                    _ => {}
                                }
                           },
                           _ => {}
                       }
                    },  
                    None => {}
                };
            }
            event = indexer_event_receiver.recv() => {
                match event {
                    Some(out) => {
                        use indexer::IndexerEvent;
                        match out {
                            IndexerEvent::PlaceBid(bid) => {
                                match self.network_client.send_dse_message_request()
                            }
                        }
                    },
                    None => {}
                }
            },
            line = stdin.next() => {    
                match line.expect("Line buffer errored") {
                    Ok(l) => {
                        handle_input(l, &mut client).await;
                    }
                    Err(e) => {}
                }
            }
        }
    }

    Ok(())
}

async fn handle_input(command: String, client: &mut network::Client) {
    let mut args = command.split(" ");

    match args.next() {
        Some("BOOTSTRAP") => {
            match client.kad_bootstrap().await {
                Ok(()) => {
                    println!("BOOTSTRAP success!");
                },
                Err(e) => {
                    println!("BOOTSTRAP failed with errpr {:?} ", e);
                }
            }
        },
        Some("PUT") => {
            let key = {
                match args.next() {
                    Some(key) => Key::new(&key),
                    None => {
                        eprintln!("PUT record key missing!");
                        return
                    }                    
                }
            };
            let value = {
                match args.next() {
                    Some(val) => val.into(),
                    None => {
                        eprintln!("PUT record value missing!");
                        return
                    }                    
                }
            };
            
            match client.dht_put(Record {
                key,
                value,
                publisher: None,
                expires: None,
            }, Quorum::One).await {
                Ok(put_record) => {
                    println!("Put record success {:?} ", put_record);
                }
                Err(e) => {
                    println!("Put record failed {:?} ", e);
                }
            }
        },
        Some("GET") => {
            let key = {
                match args.next() {
                    Some(key) => Key::new(&key),
                    None => {
                        eprintln!("GET record key missing");
                        return
                    }
                }
            };

            match client.dht_get(key, Quorum::One).await {
                Ok(get_record) => {
                    println!("Get record success {:?} ", get_record);
                },
                Err(e) => {
                    println!("Get record failed {:?} ", e);
                }
            }
        },
        Some("PUBLISH") => {
            let message = {
                match args.next() {
                    Some(val) => val,
                    None =>{
                        eprintln!("PUBLISH message missing!");
                        return
                    }
                }
            };

            let gossip_message = network::GossipsubMessage::SearchQuery { query: message.to_string(), metadata: message.to_string() };

            match client.publish_message(gossip_message).await {
                Ok(_) => {
                    println!("Message published!");
                },
                Err(e) => {
                    eprintln!("Message failed to publish with error {:?}!", e);
                }
            }
        }
        
        _ => {
            println!("Unrecognised command!");
        }
    }
}
