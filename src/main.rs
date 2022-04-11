use async_std::io::prelude::BufReadExt;
use async_std::prelude::StreamExt;
use async_std::{self};
use libp2p::gossipsub::{self};
use libp2p::kad::record::Key;
use libp2p::kad::{Quorum, Record};
use libp2p::mdns::MdnsEvent;
use libp2p::{Multiaddr, PeerId};
use log::{debug, error, info};
use std::error;
use structopt::StructOpt;
use tokio::{select, sync::mpsc};

mod commit_procedure;
mod ethnode;
mod indexer;
mod network;
mod network_client;
mod server;
mod storage;
mod subscription;

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

    #[structopt(long)]
    server_port: u16,
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let opt = Opt::from_args();

    let eth_node =
        ethnode::EthNode::new(opt.eth_rpc_endpoint, opt.private_key, opt.wallet_address)?;
    let (mut network_event_receiver, network_interface) =
        network::new(opt.seed, network_c_receiver)
            .await
            .expect("network::new failed");
    let database = database::new(network_interface.keypair.public().to_peer_id().to_base58());
    let (indexer_event_receiver, indexer_interface) = indexer::new(
        network_client.clone(),
        commitment_client.clone(),
        indexer_c_receiver,
        database.clone(),
    );
    let commitment_interface = commitment::new(
        commitment_c_receiver,
        network_client.clone(),
        eth_node.clone(),
    );
}
