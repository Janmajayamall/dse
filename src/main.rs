use async_std::io;
use async_std::prelude::StreamExt;
use libp2p::core::transport::Boxed;
use libp2p::futures::stream::Peek;
use libp2p::kad::{GetProvidersOk, Kademlia, KademliaEvent, QueryId, QueryResult, KademliaConfig, KadConnectionType};
use libp2p::kad::record::store::MemoryStore;
use libp2p::{NetworkBehaviour, PeerId, Transport, Multiaddr};
use libp2p::swarm::{NetworkBehaviour, NetworkBehaviourEventProcess, SwarmBuilder, SwarmEvent};
use libp2p::identity::{Keypair};
use libp2p::dns::TokioDnsConfig;
use libp2p::tcp::TokioTcpConfig;
use libp2p::core::transport::upgrade::Version;
use libp2p::core::upgrade::SelectUpgrade;
use libp2p::core::muxing::StreamMuxerBox;
use libp2p::yamux::YamuxConfig;
use libp2p::mplex::MplexConfig;
use tokio::{select};
use libp2p::noise;
use std::error;
use std::time::Duration;


#[derive(NetworkBehaviour)]
#[behaviour(event_process = true)]
pub struct Behaviour {
    kademlia: Kademlia<MemoryStore>
}

impl NetworkBehaviourEventProcess<KademliaEvent> for Behaviour {

    fn inject_event(&mut self, event: KademliaEvent) {
        match event {
            KademliaEvent::RoutingUpdated { peer, is_new_peer ,addresses, bucket_range, old_peer } => {
                println!(
                    "RoutingUpdated with peerId {:?} address {:?} ", peer, addresses
                );
            }
            _ => {

            }
        }
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
        let mut kademlia = Kademlia::with_config(peer_id, store, kad_config);


        // add bootnode
        let bootnode_addr : Multiaddr = "/ip4/127.0.0.1/tcp/45195".parse().expect("Invalid boot node address");
        let bootnode_id : PeerId = "16Uiu2HAkw4YUiioa9rCwVcgamNP1SkroWkgK433QWF2Ujt7ATGDX".parse().expect("Invalida boot node peer id");
        kademlia.add_address(&bootnode_id, bootnode_addr);

        // bootstrap node
        match kademlia.bootstrap() {
            Ok(id) => {
                println!("Bootstrap query id {:?} ", id);
            }
            Err(e) => {
                println!("No known peers");
            }
        }

        Behaviour {
            kademlia,
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
    // create keypair for the node
    let keypair = Keypair::generate_secp256k1();
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


    // Listen on all interfaces and whatever port the OS assigns.
    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

   
    loop {
        select! {
            event = swarm.next() => {
                println!("Swarm event {:?} ", event);
            }
        }
    }

    Ok(())

}


// match event.expect("Event errored") {
//                 SwarmEvent::NewListenAddr {listener_id , address} => {
//                     println!("New address {:?} reported by {:?} ", listener_id, address);
//                 }
//                 SwarmEvent::ConnectionEstablished {peer_id, endpoint, num_established, concurrent_dial_errors} => {
//                     println!("Connection establised");
//                 }
//                 _ => {
//                     println!("YO {:?} ", event);
//                 }
//             }