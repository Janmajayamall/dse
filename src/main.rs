use async_std::io;
use libp2p::kad::{GetProvidersOk, Kademlia, KademliaEvent, QueryId, QueryResult};
use libp2p::kad::record::store::MemoryStore;
use libp2p::{NetworkBehaviour};
use libp2p::swarm::{NetworkBehaviour, NetworkBehaviourEventProcess};
use libp2p::identity::{Keypair};
use std::error::Error;

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

#[async_std::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // create keypair for the node
    let keypair = Keypair::generate_secp256k1();
    let peer_id = keypair.public().to_peer_id();

    // connect

    Ok(())

}