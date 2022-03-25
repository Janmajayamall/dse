use serde::{Deserialize, Serialize};

use super::network;

pub type QueryId = [u8; 32];

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchQuery {
    id: QueryId,
    query: String,
    metadata: String,
    expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PlaceBid {
    query_id: QueryId,
    bid: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AckBid {
    query_id: QueryId
}


pub fn handle_search_query(query: SearchQuery) {
        
}

pub fn handle_bid(bid: &PlaceBid) { 
    
}

pub async fn handle_network_event(event: network::NetworkEvent, network_client: &mut network::Client) {
    use network::{NetworkEvent, GossipsubMessage, DseMessageRequest, DseMessageResponse};
    match event {
        NetworkEvent::GossipsubMessageRecv(message)  => {
            match message { 
                GossipsubMessage::SearchQuery(query) => {
                    handle_search_query(query);
                }
            }
        },
        NetworkEvent::DseMessageRequestRecv { request_id, request, channel } => {
            match request { 
                DseMessageRequest::PlaceBid(bid) => {
                    handle_bid(&bid);

                    // TODO Send ack depending on the expriy time
                    match network_client.send_dse_message_response(request_id, channel, network::DseMessageResponse::AckBid(AckBid {
                        query_id: bid.query_id
                    })).await {
                        Ok(_) => {},
                        Err(_) => {
                            // TODO probably retry sending ACK depending on the error
                            // type thrown
                        }
                    }
                }   
            }
        },
        _ => {}
    }
}


pub fn handle_received_query(query: network::GossipsubMessage) {
    // As of now the plan is to 
    // forward every query that is received by gossipsub
    // to the RPC endpoint.
    // That mean enpoint might receive
    // duplicate or expird search queries
}