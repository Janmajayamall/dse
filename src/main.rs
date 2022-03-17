use async_std::io;
use libp2p::kad::{GetProvidersOk, Kademlia, KademliaEvent, QueryId, QueryResult, KademliaConfig, KadConnectionType};
use libp2p::kad::record::store::MemoryStore;
use libp2p::{NetworkBehaviour, PeerId};
use libp2p::swarm::{NetworkBehaviour, NetworkBehaviourEventProcess};
use libp2p::identity::{Keypair};
use std::error::Error;
use std::time::Duration;

#[derive(NetworkBehaviour)]
#[behaviour(event_process = true)]
pub struct Behaviour {
    kademlia: Kademlia<MemoryStore>
}

impl NetworkBehaviourEventProcess<KademliaEvent> for Behaviour {
    fn inject_event(&mut self, event: KademliaEvent) {
        //TODO process kedamile events
    }
}

impl Behaviour {
    pub async fn new(peer_id: PeerId)  {
        // setup kademlia
        let store = MemoryStore::new(peer_id);
        let mut kad_config = KademliaConfig::default();
        kad_config.set_protocol_name("kad_protocol".as_bytes());
        kad_config.set_query_timeout(Duration::from_secs(300));
        // TODO set disjoint_query_paths to true, once you understand this -https://discuss.libp2p.io/t/s-kademlia-lookups-over-disjoint-paths-in-rust-libp2p/571
        let kademila = Kademlia::with_config(peer_id, store, kad_config);

        // TODO supply and add bootstrap nodes
        
        

    }    
}

#[async_std::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // create keypair for the node
    let keypair = Keypair::generate_secp256k1();
    let peer_id = keypair.public().to_peer_id();

    // connect



    Ok(())

}