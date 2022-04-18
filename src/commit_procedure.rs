use super::ch_request;
use super::network_client;
use super::storage::{self, TradeStatus};
use crate::ethnode::EthNode;
use crate::network::ExchangeRequest;
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
    SendSuccess { trade_id: u32 },
    SendFailed { trade_id: u32 },
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
        // find wallet's ownet address
        let owner_address = self.ethnode.owner_address(&commit.wallet_address).await;

        // Verify that the commit is valid for the trade.
        if !commit.is_commit_valid(&self.trade, &owner_address) {
            // TODO: Behaviour when a invalid commit
            // received is still unknown.
            // I suggest that we cancel the entire trade,
            // so that we don't have to deal with complexity
            // of recovery mechanism. Also, the peer is dishonest anyways
            // so cutting off the trade makes sense.
        }

        // update trade status to suitable processing status
        self.update_trade_processing_status();

        // If commit history of wallet does not exists,
        // then first fetch it.
        if self
            .storage
            .find_commit_history(&commit.wallet_address)
            .is_none()
        {
            let mut req = ch_request::ChRequest {
                network_client: self.network_client.clone(),
                storage: self.storage.clone(),
                ethnode: self.ethnode.clone(),
            };

            req.load_commit_history(&commit.wallet_address).await;
        }

        match self.storage.find_commit_history(&commit.wallet_address) {
            Some(commit_history) => {
                // TODO: Cancel trade if indexes are in conflict?
                // Or ask for invalidatation signature if the commit
                // in conflict type c1?
                commit_history.are_indexes_in_conflict(&commit.indexes);
            }
            None => {}
        }

        // Update trade status suitable send status
        self.update_trade_send_status();

        // // Perform commit procedure
        // // Simulates verification
        // let mut interval = time::interval(time::Duration::from_secs(10));
        // interval.tick().await;
        // interval.tick().await;
    }

    /// Used for sending commits to peer for a trade  
    ///
    /// Should make sure that trade is in send status
    /// before calling this
    pub async fn send(mut self) {
        // Before calling `send` make sure that self.trade
        // is in valid send state
        let commit = self
            .find_commitment()
            .expect("Invalid state OR failed to find valid commit");

        // send commit to peer
        match self
            .network_client
            .send_exchange_request(
                self.trade.counter_party_details().0,
                self.create_send_request(commit)
                    .expect("Trade status isn't SOMETHING SEND"),
            )
            .await
        {
            Ok(_) => {
                // update status from send to suitable waiting
                self.update_trade_waiting_status();

                self.event_sender
                    .send(CommitProcedureEvent::SendSuccess {
                        trade_id: self.trade.id,
                    })
                    .await;
            }
            Err(_) => {
                let _ = self
                    .event_sender
                    .send(CommitProcedureEvent::SendFailed {
                        trade_id: self.trade.id,
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
    fn create_send_request(&self, commit: storage::Commit) -> Option<ExchangeRequest> {
        match self.trade.status {
            TradeStatus::RSendT1Commit => Some(ExchangeRequest::T1RequesterCommit {
                trade_id: self.trade.id,
                commit,
            }),
            TradeStatus::PSendT1Commit => Some(ExchangeRequest::T1ProviderCommit {
                trade_id: self.trade.id,
                commit,
            }),
            TradeStatus::RSendT2Commit => Some(ExchangeRequest::T2RequesterCommit {
                trade_id: self.trade.id,
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
