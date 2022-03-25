use tokio::sync::{mpsc, oneshot};
use tokio::{select};

type QueryId = [u8; 32];

pub struct Bid {
    query_id: QueryId,
    bid: String,
}

pub struct SearchQuery {
    id: QueryId,
    query: String,
    metadata: String,
    expires_at: chrono::DateTime<chrono::Utc>,
}

pub enum Command {
    HandleReceivedBid {
        bid: Bid, 
    },
    HandleReceivedSearchQuery{
        query: SearchQuery,
    }
}

pub enum IndexerEvent {
    PlaceBid { 

    },
    NewSearch {

    },
}


pub struct Client {
    command_sender: mpsc::Sender<Command>,
}

// Client receives commands and forwards them 
impl Client {
    pub async fn handle_received_bid() -> Result<(), anyhow::Error> {
        
    } 
}


/// Main interface thru which user interacts.
/// That means sends and receives querues & bids.
struct Indexer {
    command_receiver: mpsc::Receiver<Command>,
}

impl Indexer {
    pub fn new(
        command_receiver: mpsc::Receiver<Command>,
    ) -> Self {
        Self {
            command_receiver,
        }
    }

    pub async fn run(mut self) {
        loop {
            select! {
                command = self.command_receiver.recv() => {
                    match command {
                        Some(c) => self.command_handler(c).await,
                        None => {}
                    }
                }
            }
        }
    }

    pub async fn command_handler(&mut self, command: Command) {

    }

}

// Note - 
// processing of queries and bids
// should run on different 
// thread (or atleast independent 
// of other things). Therefore, 
// a channel is needed for notifying
// main.

// Node - 
// Create a dummy indexer that sends
// queries/bids randomly.
