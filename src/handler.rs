mod network;


pub type SearchId = [u8; 32];


pub fn handle_received_query(query: network::GossipsubMessage::SearchQuery) {
    // As of now the plan is to 
    // forward every query that is received by gossipsub
    // to the RPC endpoint.
    // That mean enpoint might receive
    // duplicate or expird search queries
}