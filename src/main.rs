use async_std::io::prelude::BufReadExt;
use async_std::{self};
use async_std::prelude::StreamExt;
use libp2p::kad::record::Key;
use libp2p::kad::{Quorum, Record};
use libp2p::{PeerId, Multiaddr};
use libp2p::mdns::{MdnsEvent};
use libp2p::gossipsub::{self};
use log::{debug, error, info};
use tokio::{select, sync::{mpsc}};
use std::error;
use structopt::StructOpt;

mod network;
mod indexer;
mod server;
mod ethnode;
mod commitment;

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
    boot_addr: Option<Multiaddr>,
    
    #[structopt(long)]
    eth_rpc_endpoint: String,
    
    #[structopt(long)]
    private_key: String,

    #[structopt(long)]
    wallet_address: ethers::types::H160,
}


#[tokio::main]
async fn main() -> Result<(), Box<dyn error::Error>> {
    env_logger::init();

    let opt = Opt::from_args();

    // create clients
    let (
        (mut network_client, network_c_receiver),
        (mut indexer_client, indexer_c_receiver),
        (mut commitment_client, commitment_c_receiver)
    ) = create_clients();

    let eth_node = ethnode::EthNode::new(opt.eth_rpc_endpoint, opt.private_key, opt.wallet_address)?;

    let (mut network_event_receiver, network_interface) = network::new(opt.seed, network_c_receiver).await.expect("network::new failed");
    let (indexer_event_receiver, indexer_interface, server_event_sender) = indexer::new(network_client.clone(), commitment_client.clone(), indexer_c_receiver);
    let commitment_interface = commitment::new(commitment_c_receiver, network_client.clone(), eth_node.clone());
    

    tokio::spawn(network_interface.run());
    tokio::spawn(indexer_interface.run());
    tokio::spawn(commitment_interface.run());
    tokio::spawn(server::new(server_event_sender));

    // Listen on either provided opt value or any interface
    if let Some(listen_on) = opt.listen_on {
        let _ = network_client.start_listening(listen_on).await;
    }else {
        let _ = network_client.start_listening("/ip4/0.0.0.0/tcp/0".parse()?).await;
    }

    // add bootnode
    if let (Some(boot_id),  Some(boot_addr)) = (opt.boot_id, opt.boot_addr) {
        info!("Bootnode added with peerId {:?} multiAddr {:?}", boot_id, boot_addr);
        let _ = network_client.add_kad_peer(boot_id, boot_addr).await;
    }

    // read std input
    let mut stdin = async_std::io::BufReader::new(async_std::io::stdin()).lines();

    let mut node_multiaddress: Option<Multiaddr> = None;

    loop {
        select! {
            event = network_event_receiver.recv() => {
                match event {
                    Some(out) => {
                        // println!("NetworkEvent: received {:?} ", out);
                        match out {
                           network::NetworkEvent::NewListenAddr {
                               listener_id,
                               address
                           } => {
                                node_multiaddress = Some(address);
                           },
                           network::NetworkEvent::Mdns(MdnsEvent::Discovered(list)) => {
                                for node in list {
                                    println!("Discovered node with peer id {:?} and multiaddr {:?} ", node.0, node.1);
                                    network_client.add_kad_peer(node.0, node.1).await.expect("Client fn call dropped");
                                }
                           },
                           network::NetworkEvent::GossipsubMessageRecv(message) => {
                                match message {
                                    network::GossipsubMessage::NewQuery(query) => {
                                        // TODO handle error
                                        let _ = indexer_client.handle_received_query(query).await;
                                    },
                                    _ => {}
                                }
                           },
                           network::NetworkEvent::DseMessageRequestRecv {peer_id, request_id, request} => {
                                match request {
                                    network::DseMessageRequest::PlaceBid(bid_recv) => {
                                        // send AckBid response
                                        // TODO handle case when send dse message
                                        // response fails.
                                        let _  = network_client.send_dse_message_response(request_id, network::DseMessageResponse::AckBid(bid_recv.query_id)).await;

                                        // TODO handle in case or err
                                        let _ = indexer_client.handle_received_bid(bid_recv).await;
                                    },
                                    network::DseMessageRequest::AcceptBid(query_id) => {
                                        // send ack
                                        // TODO handle err
                                        let _ = network_client.send_dse_message_response(request_id, network::DseMessageResponse::AckAcceptBid(query_id)).await;

                                        let _ = indexer_client.handle_received_bid_acceptance(query_id, peer_id).await;
                                    },
                                    network::DseMessageRequest::StartCommit(query_id) => {
                                        // send ack
                                        let _ = network_client.send_dse_message_response(request_id, network::DseMessageResponse::AckStartCommit(query_id)).await;

                                        let _ = indexer_client.handle_received_start_commit(query_id, peer_id).await;

                                    },
                                    
                                    _ => {}
                                }
                           },
                           _ => {}
                       }
                    },  
                    None => {}
                };
            },
            // event = indexer_event_receiver.recv() => {
            //     // match event {
            //     //     Some(out) => {
            //     //         // use indexer::IndexerEvent;
            //     //         match out {
            //     //             indexer::IndexerEvent::SendDseMessageRequest{
            //     //                 request,
            //     //                 send_to
            //     //             } => {   
            //     //                 // TODO handle eerror
            //     //                 let _ = network_client.send_dse_message_request(send_to, request).await;
            //     //             },
            //     //             indexer::IndexerEvent::NewQuery {
            //     //                 query
            //     //             } => {

            //     //             },
            //     //             indexer::IndexerEvent::RequestNodeMultiAddr{
            //     //                 sender
            //     //             } => {  
            //     //                 match node_multiaddress {
            //     //                     Some(add) => {
            //     //                         sender.send(Ok(add));
            //     //                     },
            //     //                     None => {
            //     //                         sender.send(Err(anyhow::anyhow!("indexer event: Node multi addr does not exist!")));
            //     //                     },
            //     //                 }
            //     //             },
            //     //             _ => {}
            //     //         }
            //     //     },
            //     //     None => {}
            //     // }
            // },
            line = stdin.next() => {    
                // match line.expect("Line buffer errored") {
                //     Ok(l) => {
                //         // handle_input(l, &mut client).await;
                //     }
                //     Err(e) => {}
                // }
            }
        }
    }

    Ok(())
}

fn create_clients() -> (
    (network::Client, mpsc::Receiver<network::Command>),
    (indexer::Client, mpsc::Receiver<indexer::Command>),
    (commitment::Client, mpsc::Receiver<commitment::Command>)
) {
    let (network_sender, network_receiver) = mpsc::channel::<network::Command>(10);
    let (indexer_sender, indexer_receiver) = mpsc::channel::<indexer::Command>(10);
    let (commitment_sender, commitment_receiver) = mpsc::channel::<commitment::Command>(10);
    return (
        (network::Client{command_sender: network_sender}, network_receiver),
        (indexer::Client{command_sender: indexer_sender}, indexer_receiver),
        (commitment::Client{command_sender: commitment_sender}, commitment_receiver),
    );
}

async fn handle_input(command: String, client: &mut network::Client) {
    let mut args = command.split(" ");

    // match args.next() {
    //     Some("BOOTSTRAP") => {
    //         match client.kad_bootstrap().await {
    //             Ok(()) => {
    //                 println!("BOOTSTRAP success!");
    //             },
    //             Err(e) => {
    //                 println!("BOOTSTRAP failed with errpr {:?} ", e);
    //             }
    //         }
    //     },
    //     Some("PUT") => {
    //         let key = {
    //             match args.next() {
    //                 Some(key) => Key::new(&key),
    //                 None => {
    //                     eprintln!("PUT record key missing!");
    //                     return
    //                 }                    
    //             }
    //         };
    //         let value = {
    //             match args.next() {
    //                 Some(val) => val.into(),
    //                 None => {
    //                     eprintln!("PUT record value missing!");
    //                     return
    //                 }                    
    //             }
    //         };
            
    //         match client.dht_put(Record {
    //             key,
    //             value,
    //             publisher: None,
    //             expires: None,
    //         }, Quorum::One).await {
    //             Ok(put_record) => {
    //                 println!("Put record success {:?} ", put_record);
    //             }
    //             Err(e) => {
    //                 println!("Put record failed {:?} ", e);
    //             }
    //         }
    //     },
    //     Some("GET") => {
    //         let key = {
    //             match args.next() {
    //                 Some(key) => Key::new(&key),
    //                 None => {
    //                     eprintln!("GET record key missing");
    //                     return
    //                 }
    //             }
    //         };

    //         match client.dht_get(key, Quorum::One).await {
    //             Ok(get_record) => {
    //                 println!("Get record success {:?} ", get_record);
    //             },
    //             Err(e) => {
    //                 println!("Get record failed {:?} ", e);
    //             }
    //         }
    //     },
    //     Some("PUBLISH") => {
    //         let message = {
    //             match args.next() {
    //                 Some(val) => val,
    //                 None =>{
    //                     eprintln!("PUBLISH message missing!");
    //                     return
    //                 }
    //             }
    //         };

    //         let gossip_message = network::GossipsubMessage::SearchQuery { query: message.to_string(), metadata: message.to_string() };

    //         match client.publish_message(gossip_message).await {
    //             Ok(_) => {
    //                 println!("Message published!");
    //             },
    //             Err(e) => {
    //                 eprintln!("Message failed to publish with error {:?}!", e);
    //             }
    //         }
    //     }
        
    //     _ => {
    //         println!("Unrecognised command!");
    //     }
    // }


}
