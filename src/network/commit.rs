use super::request_response::RequestResponse;
use super::storage;
use libp2p::core::ProtocolName;
use serde::{Deserialize, Serialize};

pub const COMMIT_PROTOCOL_ID: &[u8] = b"/dse/commit/0.1";

#[derive(Clone)]
pub struct CommitProtocol;

impl ProtocolName for CommitProtocol {
    fn protocol_name(&self) -> &[u8] {
        COMMIT_PROTOCOL_ID
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum CommitRequest {
    WantHistory {
        wallet_address: ethers::types::Address,
    },
    Update {
        wallet_address: ethers::types::Address,
        commits: Vec<storage::Commit>,
        last_batch: bool,
    },
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum CommitResponse {
    /// Acknowledges the request
    Ack,
    /// Bad request
    Bad,
}

pub type CommitCodec = RequestResponse<CommitProtocol, CommitRequest, CommitResponse>;
