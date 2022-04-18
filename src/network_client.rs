use super::network::{self, Command};
use anyhow::anyhow;
use libp2p::kad::{
    record, AddProviderOk, Addresses, BootstrapError, GetProvidersOk, GetRecordError, GetRecordOk,
    Kademlia, KademliaConfig, KademliaEvent, PutRecordOk, QueryId, QueryResult, Quorum, Record,
};
use libp2p::{gossipsub, request_response::RequestId, Multiaddr, PeerId, Transport};
use tokio::sync::{broadcast, mpsc, oneshot};

// client & event loop for network
#[derive(Clone)]
pub struct Client {
    pub command_sender: mpsc::Sender<Command>,
}

impl Client {
    pub fn new(command_sender: mpsc::Sender<Command>) -> Self {
        Self { command_sender }
    }

    pub async fn add_kad_peer(
        &mut self,
        peer_id: PeerId,
        peer_addr: Multiaddr,
    ) -> anyhow::Result<()> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::AddKadPeer {
                peer_id,
                peer_addr,
                sender,
            })
            .await?;
        receiver.await.map_err(anyhow::Error::from)
    }

    pub async fn add_request_response_peer(
        &self,
        peer_id: PeerId,
        peer_addr: Multiaddr,
    ) -> anyhow::Result<()> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::AddRequestResponsePeer {
                peer_id,
                peer_addr,
                sender,
            })
            .await?;
        receiver.await.map_err(anyhow::Error::from)
    }

    pub async fn start_listening(&mut self, addr: Multiaddr) -> anyhow::Result<()> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::StartListening { addr, sender })
            .await?;
        receiver.await?
    }

    pub async fn dht_put(&mut self, record: Record, quorum: Quorum) -> anyhow::Result<PutRecordOk> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::DhtPut {
                record,
                quorum,
                sender,
            })
            .await?;
        receiver.await?
    }

    pub async fn dht_get(
        &mut self,
        key: record::Key,
        quorum: Quorum,
    ) -> anyhow::Result<GetRecordOk> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::DhtGet {
                key,
                quorum,
                sender,
            })
            .await?;
        receiver.await?
    }

    pub async fn dht_start_providing(&self, key: record::Key) -> anyhow::Result<AddProviderOk> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::DhtStartProviding { key, sender })
            .await?;
        receiver.await?
    }

    pub async fn dht_get_providers(&self, key: record::Key) -> anyhow::Result<GetProvidersOk> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::DhtGetProviders { key, sender })
            .await?;
        receiver.await?
    }

    pub async fn kad_bootstrap(&mut self) -> anyhow::Result<()> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::Bootstrap { sender })
            .await?;
        receiver.await?
    }

    pub async fn publish_message(
        &mut self,
        message: network::GossipsubMessage,
    ) -> Result<gossipsub::MessageId, anyhow::Error> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::PublishMessage { message, sender })
            .await?;
        receiver.await?
    }

    pub async fn send_exchange_request(
        &self,
        peer_id: PeerId,
        request: network::ExchangeRequest,
    ) -> Result<network::ExchangeResponse, anyhow::Error> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::SendExchangeRequest {
                peer_id,
                request,
                sender,
            })
            .await?;
        receiver.await?
    }

    pub async fn send_exchange_response(
        &mut self,
        peer_id: PeerId,
        request_id: RequestId,
        response: network::ExchangeResponse,
    ) -> Result<(), anyhow::Error> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::SendExchangeResponse {
                peer_id,
                request_id,
                response,
                sender,
            })
            .await?;
        receiver.await?
    }

    pub async fn send_commit_request(
        &self,
        peer_id: PeerId,
        request: network::CommitRequest,
    ) -> Result<network::CommitResponse, anyhow::Error> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::SendCommitRequest {
                peer_id,
                request,
                sender,
            })
            .await?;
        receiver.await?
    }

    pub async fn send_commit_response(
        &mut self,
        peer_id: PeerId,
        request_id: RequestId,
        response: network::CommitResponse,
    ) -> Result<(), anyhow::Error> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::SendCommitResponse {
                peer_id,
                request_id,
                response,
                sender,
            })
            .await?;
        receiver.await?
    }

    pub async fn network_details(&mut self) -> Result<(PeerId, Multiaddr), anyhow::Error> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::NetworkDetails { sender })
            .await?;
        receiver.await?
    }

    pub async fn subscribe_network_events(
        &self,
    ) -> Result<broadcast::Receiver<network::NetworkEvent>, anyhow::Error> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::SubscribeNetworkEvents { sender })
            .await;
        Ok(receiver.await?)
    }
}
