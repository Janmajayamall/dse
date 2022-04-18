use async_std::sync::Arc;
use libp2p::{
    identity::{secp256k1, Keypair},
    Multiaddr, PeerId,
};
use log::{debug, error, info};
use structopt::StructOpt;
use tokio::select;

mod ch_request;
mod commit_procedure;
mod ethnode;
mod indexer;
mod network;
mod network_client;
mod server;
mod storage;

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

    let keypair = {
        match opt.seed {
            Some(seed) => {
                let mut bytes: [u8; 32] = [0; 32];
                bytes[0] = seed;
                Keypair::Secp256k1(
                    secp256k1::SecretKey::from_bytes(bytes)
                        .expect("Invalid keypair seed")
                        .into(),
                )
            }
            _ => Keypair::generate_secp256k1(),
        }
    };

    debug!("Node peer_id {}", keypair.public().to_peer_id());

    let bootstrap_peers = {
        if let Some((peer_id, addr)) = opt
            .boot_addr
            .and_then(|addr| opt.boot_id.and_then(|peer_id| Some((peer_id, addr))))
        {
            vec![(peer_id, addr)]
        } else {
            Vec::<(PeerId, Multiaddr)>::new()
        }
    };

    let listen_on: Multiaddr = match opt.listen_on {
        Some(addr) => addr,
        None => "/ip4/0.0.0.0/tcp/0"
            .parse()
            .expect("Default listen on addr parse errored"),
    };
    let network = network::Network::new(keypair.clone(), bootstrap_peers, listen_on)
        .await
        .expect("Network startup failed!");

    let network_client = network_client::Client::new(network.network_command_sender());

    let ethnode = ethnode::EthNode::new(opt.eth_rpc_endpoint, opt.private_key, opt.wallet_address)
        .expect("EthNode failed to start");

    let storage = Arc::new(storage::Storage::new(keypair.public().to_peer_id()));

    let indexer = indexer::Indexer::new(
        keypair.clone(),
        network_client.clone(),
        network.network_event_receiver(),
        storage.clone(),
        ethnode.clone(),
    );

    let server = server::Server::new(
        network_client.clone(),
        storage.clone(),
        ethnode.clone(),
        keypair.clone(),
        opt.server_port,
    );

    tokio::spawn(async {
        server.start().await;
    });

    let mut waiter = network.network_event_receiver();
    tokio::spawn(async {
        network.run().await;
    });

    tokio::spawn(async {
        indexer.run().await;
    });

    // FIXME: replace this with waiting for ctrl+c
    loop {
        select! {
            e = waiter.recv() => {

            }
        }
    }
}
