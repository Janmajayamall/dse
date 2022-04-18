use super::request_response::RequestResponse;
use super::storage;
use libp2p::core::ProtocolName;
use serde::{Deserialize, Serialize};

pub const EXCHANGE_PROTOCOL_ID: &[u8] = b"/dse/exchange/0.1";

#[derive(Clone)]
pub struct ExchangeProtocol;

impl ProtocolName for ExchangeProtocol {
    fn protocol_name(&self) -> &[u8] {
        EXCHANGE_PROTOCOL_ID
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum ExchangeRequest {
    /// Requester Sends IWant to Provider
    IWant { i_want: storage::IWant },
    /// Provider sends start commit procedure
    /// to Requester along with wallet address.
    StartCommit { trade: storage::Trade },
    /// Requester sends T1 commitment to Provider
    ///
    /// This request acknowledges that requester's
    /// wallet address is suitable for exchange
    T1RequesterCommit {
        trade_id: u32,
        commit: storage::Commit,
    },
    /// Provider sends T1 commitment to Requester
    ///
    /// This request acknowledges that Requester's T1
    /// commitment is valid
    T1ProviderCommit {
        trade_id: u32,
        commit: storage::Commit,
    },
    /// Requester sends T2 commitment to Provider
    ///
    /// This request acknowledges that Provider's T1
    /// commitment is valid
    T2RequesterCommit {
        trade_id: u32,
        commit: storage::Commit,
    },
    /// Provider sends acknowledgement to Requester
    /// that T2 commit is valid
    T2CommitValid { trade_id: u32 },
    /// Requester sends invaldating signature for commits
    /// to the Provider
    InvalidatingSignature {
        trade_id: u32,
        invalidating_signature: ethers::types::Signature,
    },
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
pub enum ExchangeResponse {
    /// Acknowledges the request
    Ack,
    /// Bad request
    Bad,
}

pub type ExchangeCodec = RequestResponse<ExchangeProtocol, ExchangeRequest, ExchangeResponse>;
