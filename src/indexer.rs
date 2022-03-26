use tokio::sync::{mpsc, oneshot};
use tokio::{select};


type QueryId = [u8; 32];

#[derive(Debug)]
pub struct Bid {
    query_id: QueryId,
    bid: String,
}

#[derive(Debug)]
pub struct SearchQuery {
    id: QueryId,
    query: String,
    metadata: String,
    expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug)]
pub enum Command {
    HandleReceivedBid(Bid), 
    HandleReceivedSearchQuery {
        query: SearchQuery,
        sender: oneshot::Sender<Result<(), anyhow::Error>>,
    },
}

#[derive(Debug)]
pub enum IndexerEvent {
    PlaceBid(Bid),
    NewSearch(SearchQuery),
}


pub struct Client {
    command_sender: mpsc::Sender<Command>,
}

// Client receives commands and forwards them 
impl Client {
    pub async fn handle_received_bid() -> Result<(), anyhow::Error> {
        Ok(())
    } 

    pub async fn handle_received_query(&mut self, query: SearchQuery) -> Result<(), anyhow::Error> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender.send(
            Command::HandleReceivedSearchQuery {
                query,
                sender,
            }
        ).await.expect("Command message dropped");
        receiver.await.expect("Command response dropped")
    }
}


/// Main interface thru which user interacts.
/// That means sends and receives querues & bids.
struct Indexer {
    command_receiver: mpsc::Receiver<Command>,
    event_sender: mpsc::Sender<IndexerEvent>
}

impl Indexer {
    pub fn new(
        command_receiver: mpsc::Receiver<Command>,
        event_sender: mpsc::Sender<IndexerEvent>,
    ) -> Self {
        Self {
            command_receiver,
            event_sender,
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
        match command {
            Command::HandleReceivedSearchQuery { query, sender } => {
                // TODO something with the query
                sender.send(Ok(()));
            },
            _ => {}
        }
    }

    // Indexer events would come from RPC endpoint
    pub async fn event_handler(&mut self, event: IndexerEvent) {
        self.event_sender.send(event).await.expect("Indexer event message dropped!");
    }
}

pub fn new() -> (Client, mpsc::Receiver<IndexerEvent>, Indexer) {
    let (command_sender, command_receiver) = mpsc::channel::<Command>(10);
    let (indexer_event_sender, indexer_event_receiver) = mpsc::channel::<IndexerEvent>(10);

    return (
        Client {
            command_sender,
        },
        indexer_event_receiver,
        Indexer::new(command_receiver, indexer_event_sender)
    )
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
