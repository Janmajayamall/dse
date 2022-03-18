use async_std::io;
use async_std::prelude::StreamExt;
use futures::{select, StreamExt};
use libp2p::core::transport::Boxed;
use libp2p::futures::stream::Peek;
use libp2p::kad::{GetProvidersOk, Kademlia, KademliaEvent, QueryId, QueryResult, KademliaConfig, KadConnectionType};
use libp2p::kad::record::store::MemoryStore;
use libp2p::{NetworkBehaviour, PeerId};
use libp2p::swarm::{NetworkBehaviour, NetworkBehaviourEventProcess, SwarmBuilder};
use libp2p::identity::{Keypair};
use libp2p::dns::TokioDnsConfig;
use libp2p::tcp::TokioTcpConfig;
use libp2p::core::transport::upgrade::Version;
use libp2p::core::upgrade::SelectUpgrade;
use libp2p::core::muxing::StreamMuxerBox;
use libp2p::yamux::YamuxConfig;
use libp2p::mplex::MplexConfig;

use libp2p::noise;
use std::io;
use std::error;
use std::time::Duration;

#[derive(NetworkBehaviour)]
#[behaviour(event_process = true)]
pub struct Behaviour {
    kademlia: Kademlia<MemoryStore>
}

impl NetworkBehaviourEventProcess<KademliaEvent> for Behaviour {
    fn inject_event(&mut self, event: KademliaEvent) {
        println!("Received Kademlia event");
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


        // TODO supply and add bootstrap nodes

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

#[async_std::main]
async fn main() -> Result<(), Box<dyn error::Error>> {
    // create keypair for the node
    let keypair = Keypair::generate_secp256k1();
    let peer_id = keypair.public().to_peer_id();

    // connect    


    // Build swarm
    let transport = build_transport(&keypair)?;
    let behaviour = Behaviour::new(peer_id).await;
    let mut swarm = SwarmBuilder::new(
        transport,
        behaviour,
        peer_id
    ).build();


    // Listen on all interfaces and whatever port the OS assigns.
    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    loop {
        select! {
            event = swarm.select_next_some() => match event {
                _ => {
                    println!("YOo");
                }
            }
        }
    }

    Ok(())

}