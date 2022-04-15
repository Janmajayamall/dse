use super::ethnode::EthNode;
use super::network::DseMessageRequest;
use super::network_client;
use super::storage::{self, TradeStatus};
use log::error;
use std::sync::Arc;
use tokio::{sync::mpsc, time};

// responsible for check whether
pub struct CommitProcedure {
    storage: Arc<storage::Storage>,
    trade: storage::Trade,
    network_client: network_client::Client,
    ethnode: EthNode,
    event_sender: mpsc::Sender<CommitProcedureEvent>,
}

pub enum CommitProcedureEvent {
    SendSuccess { query_id: storage::QueryId },
    SendFailed { query_id: storage::QueryId },
}

impl CommitProcedure {
    pub fn new(
        storage: Arc<storage::Storage>,
        trade: storage::Trade,
        network_client: network_client::Client,
        ethnode: EthNode,
        event_sender: mpsc::Sender<CommitProcedureEvent>,
    ) -> Self {
        Self {
            storage,
            trade,
            network_client,
            ethnode,
            event_sender,
        }
    }

    /// User for verifying commit received from peer
    /// for a trade
    ///
    /// Should make sure that trade is in waiting status
    /// for self
    pub async fn verify(mut self, commit: storage::Commit) {
        // Verify that the commit is valid for the trade.
        // Check that commit amount is in accordance with
        // the status.

        // If node does not have commit history for the other node
        // then request it

        // update trade status to suitable processing status
        self.update_trade_processing_status();

        // Perform commit procedure
        // Simulates verification
        let mut interval = time::interval(time::Duration::from_secs(10));
        interval.tick().await;
        interval.tick().await;

        // Update trade status suitable send status
        self.update_trade_send_status();
    }

    /// Used for sending commits to peer for a trade
    ///
    /// Should make sure that trade is in send status
    /// before calling this
    pub async fn send(mut self) {
        // Indexer makes sure that self.trade
        // is in valid send state
        let commit = self
            .find_commitment()
            .expect("Invalid state OR failed to find valid commit");

        // send commit to peer
        let peer_id = if self.trade.is_requester {
            self.trade.bid.provider_id
        } else {
            self.trade.query.requester_id
        };
        match self
            .network_client
            .send_dse_message_request(
                peer_id,
                self.create_send_request(commit)
                    .expect("Invalid trade state"),
            )
            .await
        {
            Ok(_) => {
                // update status from send to suitable waiting
                self.update_trade_waiting_status();

                self.event_sender
                    .send(CommitProcedureEvent::SendSuccess {
                        query_id: self.trade.query_id,
                    })
                    .await;
            }
            Err(_) => {
                let _ = self
                    .event_sender
                    .send(CommitProcedureEvent::SendFailed {
                        query_id: self.trade.query_id,
                    })
                    .await;
            }
        }
    }

    /// Creates commitment according to
    /// Send status
    fn find_commitment(&self) -> Option<storage::Commit> {
        // FIXME: make this real
        match self.trade.status {
            TradeStatus::RSendT1Commit => {}
            TradeStatus::PSendT1Commit => {}
            TradeStatus::RSendT2Commit => {}
            _ => {}
        };
        Some(storage::Commit {
            indexes: Default::default(),
            epoch: Default::default(),
            u: 0,
            c_type: storage::CommitType::T1,
            i_address: Default::default(),
            r_address: Default::default(),
            signature: None,
            invalidating_signature: None,
            wallet_address: Default::default(),
        })
    }

    /// Creates DseMessageRequest that should be send
    /// according to trade status
    fn create_send_request(&self, commit: storage::Commit) -> Option<DseMessageRequest> {
        match self.trade.status {
            TradeStatus::RSendT1Commit => Some(DseMessageRequest::T1RequesterCommit {
                query_id: self.trade.query_id,
                commit,
            }),
            TradeStatus::PSendT1Commit => Some(DseMessageRequest::T1ProviderCommit {
                query_id: self.trade.query_id,
                commit,
            }),
            TradeStatus::RSendT2Commit => Some(DseMessageRequest::T2RequesterCommit {
                query_id: self.trade.query_id,
                commit,
            }),
            _ => None,
        }
    }

    /// Updates trade's status from SOMETHING
    /// Send to SOMETHING Waiting
    fn update_trade_waiting_status(&mut self) {
        match self.trade.status {
            TradeStatus::PSendT1Commit => {
                self.trade.update_status(TradeStatus::WaitingRT2Commit);
            }
            TradeStatus::RSendT1Commit => {
                self.trade.update_status(TradeStatus::WaitingPT1Commit);
            }
            TradeStatus::RSendT2Commit => {
                self.trade.update_status(TradeStatus::WaitingForService);
            }
            _ => {}
        }
        self.storage.update_active_trade(self.trade.clone());
    }

    /// Updates trade's status from SOMETHING
    /// Waiting to SOMETHING Processing
    fn update_trade_processing_status(&mut self) {
        match self.trade.status {
            TradeStatus::WaitingRT1Commit => {
                self.trade.update_status(TradeStatus::ProcessingRT1Commit);
            }
            TradeStatus::WaitingPT1Commit => {
                self.trade.update_status(TradeStatus::ProcessingPT1Commit);
            }
            TradeStatus::WaitingRT2Commit => {
                self.trade.update_status(TradeStatus::ProcessingRT2Commit);
            }
            _ => {}
        }
        self.storage.update_active_trade(self.trade.clone());
    }

    /// Updates trade's status from SOMETHING
    /// Processing to SOMETHING Send
    fn update_trade_send_status(&mut self) {
        match self.trade.status {
            TradeStatus::ProcessingRT1Commit => {
                self.trade.update_status(TradeStatus::PSendT1Commit);
            }
            TradeStatus::ProcessingPT1Commit => {
                self.trade.update_status(TradeStatus::RSendT2Commit);
            }
            TradeStatus::ProcessingRT2Commit => {
                self.trade.update_status(TradeStatus::PFulfillService);
            }
            _ => {}
        }
        self.storage.update_active_trade(self.trade.clone());
    }
}
