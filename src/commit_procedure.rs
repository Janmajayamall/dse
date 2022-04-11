use super::storage;
use std::sync::Arc;
use tokio::time;

// responsible for check whether
struct CommitProcedure {
    commit: storage::Commit,
    storage: Arc<storage::Storage>,
    trade: storage::Trade,
}

impl CommitProcedure {
    pub async fn verify(&mut self) {
        // Verify that the commit is valid for the trade.
        // Check that commit amount is in accordance with
        // the status.

        // Perforn commit procedure

        // Simulates commit procedure
        let mut interval = time::interval(time::Duration::from_secs(5));
        interval.tick().await;

        // Update trade status
        self.trade.update_waiting_status();
        self.storage.update_active_trade(self.trade.clone());
    }
}
