use super::network::Command;
use anyhow::anyhow;
use libp2p::kad::{
    Addresses, BootstrapError, GetRecordError, GetRecordOk, Kademlia, KademliaConfig,
    KademliaEvent, PutRecordOk, QueryId, QueryResult, Quorum, Record,
};
use libp2p::{gossipsub, Multiaddr, NetworkBehaviour, PeerId, Swarm, Transport};
use std::error::Error;
use tokio::sync::{mpsc, oneshot};

// client & event loop for network
#[derive(Clone)]
pub struct Client {
    pub command_sender: mpsc::Sender<Command>,
}

impl Client {
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
        &mut self,
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
        receiver.await.map_err(anyhow::Error::from)
    }

    pub async fn dht_put(&mut self, record: Record, quorum: Quorum) -> anyhow::Error<PutRecordOk> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::DhtPut {
                record,
                quorum,
                sender,
            })
            .await?;
        receiver.await.map_err(anyhow::Error::from)
    }

    pub async fn dht_get(&mut self, key: Key, quorum: Quorum) -> anyhow::Error<GetRecordOk> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::DhtGet {
                key,
                quorum,
                sender,
            })
            .await?;
        receiver.await.map_err(anyhow::Error::from)
    }

    pub async fn kad_bootstrap(&mut self) -> Result<(), anyhow::Error> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::Bootstrap { sender })
            .await?;
        receiver.await.map_err(anyhow::Error::from)
    }

    pub async fn publish_message(
        &mut self,
        message: GossipsubMessage,
    ) -> Result<gossipsub::MessageId, anyhow::Error> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::PublishMessage { message, sender })
            .await?;
        receiver.await.map_err(anyhow::Error::from)
    }

    pub async fn send_dse_message_request(
        &mut self,
        peer_id: PeerId,
        message: DseMessageRequest,
    ) -> Result<DseMessageResponse, anyhow::Error> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::SendDseMessageRequest {
                peer_id,
                message,
                sender,
            })
            .await?;
        receiver.await.map_err(anyhow::Error::from)
    }

    pub async fn send_dse_message_response(
        &mut self,
        request_id: request_response::RequestId,
        response: DseMessageResponse,
    ) -> Result<(), anyhow::Error> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::SendDseMessageResponse {
                request_id,
                response,
                sender,
            })
            .await?;
        receiver.await.map_err(anyhow::Error::from)
    }

    pub async fn network_details(&mut self) -> Result<(PeerId, Multiaddr), anyhow::Error> {
        let (sender, receiver) = oneshot::channel();
        self.command_sender
            .send(Command::NetworkDetails { sender })
            .await?;
        receiver.await.map_err(anyhow::Error::from)
    }
}
