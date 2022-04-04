use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use sled::{Config, Db};
use std::path::Path;

use super::indexer;

#[derive(Debug, Clone)]
pub struct Database {
    main: Db,
}

impl Database {
    // insert user queries
    // insert received queries
    // insert bid for a usr query

    pub fn insert_user_query(&mut self, query: &indexer::QueryReceived) {
        debug!("inserting user query {:?}", query);
        debug!(
            "serialsied query {:?} into {:?}",
            query,
            bincode::serialize(&query).unwrap()
        );
        let user_queries = self
            .main
            .open_tree(b"user_queries")
            .expect("db: failed to read user queries");
        user_queries.insert(query.id.to_le_bytes(), bincode::serialize(&query).unwrap());
    }

    pub fn insert_received_query(&mut self, query: &indexer::QueryReceived) {
        debug!("inserting received query {:?}", query);
        let user_queries = self
            .main
            .open_tree(b"user_received_queries")
            .expect("db: failed to read user received queries");
        user_queries.insert(query.id.to_le_bytes(), bincode::serialize(&query).unwrap());
    }

    pub fn find_user_queries(&self) -> Result<Vec<indexer::QueryReceived>, anyhow::Error> {
        let mut all_queries: Vec<indexer::QueryReceived> = Vec::new();
        let user_queries = self.main.open_tree(b"user_queries")?;
        for val in &user_queries {
            let (id, query) = val?;
            let query = bincode::deserialize::<indexer::QueryReceived>(&query).unwrap();
            all_queries.push(query);
        }
        Ok(all_queries)
    }

    pub fn find_recv_queries(&self) -> Result<Vec<indexer::QueryReceived>, anyhow::Error> {
        let mut all_queries: Vec<indexer::QueryReceived> = Vec::new();
        let queries = self.main.open_tree(b"user_received_queries")?;
        for val in &queries {
            let (id, query) = val?;
            let query = bincode::deserialize::<indexer::QueryReceived>(&query).unwrap();
            all_queries.push(query);
        }
        Ok(all_queries)
    }
}

pub fn new(node_id: String) -> Database {
    let mut path = "./dbs/".to_string();
    path.push_str(&node_id);
    let config = Config::new().path(path);
    Database {
        main: config.open().expect("db: Failed to open db"),
    }
}
