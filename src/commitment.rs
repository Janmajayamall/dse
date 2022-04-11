// use ethers::{types::*, utils::keccak256};
// use libp2p::request_response;
// use libp2p::{Multiaddr, PeerId};
// use log::{debug, error, info};
// use serde::{Deserialize, Serialize};
// use std::collections::{HashMap, HashSet, VecDeque};
// use std::{
//     str::FromStr,
//     sync::{Arc, Mutex},
// };
// use tokio::sync::{mpsc, oneshot};
// use tokio::{select, time};

// use super::ethnode::EthNode;
// use super::indexer;
// use super::network;
// use super::subscription;

// struct CommitmentDb {
//     /// Indexes spent on T2 commits
//     type2_spent_commits: HashMap<indexer::QueryId, Vec<Commit>>,
//     /// T1 commitments received
//     type1_recv_commitments: HashMap<indexer::QueryId, Vec<Commit>>,
//     /// T2 commitments received
//     type2_recv_commitments: HashMap<indexer::QueryId, Vec<Commit>>,
//     /// Invalidating signatures by query id
//     invalidating_signatures: HashMap<indexer::QueryId, Signature>,
// }

// enum Stage {
//     /// Waiting for countr party wallet address
//     Unitialised,
//     /// Commit rounds going on
//     Commit,
//     /// Waiting for service to go through
//     WaitingForService,
//     /// Waiting for invalidation signature
//     WaitingInvalidationSignature,
//     /// Commit Ended
//     CommitEnded,
// }

// #[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
// pub struct Commit {
//     /// index used for the commitment
//     index: u32,
//     /// epoch during which the commitment is valid
//     epoch: u32,
//     /// unique id that identifies commitment with the
//     /// corresponding query id
//     u: u32,
//     /// commit type 1 or 2
//     c_type: RoundType,
//     /// address that invalidates the commitment
//     i_address: Address,
//     /// address that can redeeem this commitment
//     /// with invalidating signature.
//     /// Only needed for c_type = 2
//     r_address: Address,
//     /// owner's signature on commitment's blob
//     signature: Option<Signature>,
//     /// invalidating signature for the commi
//     invalidating_signature: Option<Signature>,
// }

// impl Commit {
//     pub fn commit_hash(&self) -> H256 {
//         let mut blob: [u8; 160] = [0; 160];
//         U256::from(self.index).to_little_endian(&mut blob[32..64]);
//         U256::from(self.epoch).to_little_endian(&mut blob[64..96]);
//         U256::from(self.u).to_little_endian(&mut blob[96..128]);
//         U256::from(self.c_type.clone() as u32).to_little_endian(&mut blob[128..160]);

//         let mut blob: Vec<u8> = Vec::from(blob);

//         blob.extend_from_slice(self.i_address.as_bytes());

//         // r_address is added for commitment type 2
//         if self.c_type == RoundType::T2 {
//             blob.extend_from_slice(self.r_address.as_bytes());
//         }

//         keccak256(blob.as_slice()).into()
//     }
// }

// #[derive(Debug, Clone)]
// pub struct Request {
//     pub is_requester: bool,
//     pub bid: indexer::BidReceived,
//     pub query: indexer::QueryReceived,
// }

// impl Request {
//     fn counter_party_peer_id(&self) -> PeerId {
//         if self.is_requester == true {
//             return self.bid.bidder_id;
//         };
//         return self.query.requester_id;
//     }

//     fn counter_party_multiaddress(&self) -> Multiaddr {
//         if self.is_requester == true {
//             return self.bid.bidder_addr.clone();
//         };
//         return self.query.requester_addr.clone();
//     }

//     fn query_id(&self) -> indexer::QueryId {
//         self.query.id
//     }
// }

// #[derive(Debug)]
// pub enum ProcedureRequest {
//     /// Find a valid index for commitment
//     FindCommitmentIndex {
//         query_id: indexer::QueryId,
//         sender: oneshot::Sender<Result<u32, anyhow::Error>>,
//     },
// }

// #[derive(Debug)]
// pub enum ProcedureCommand {
//     /// End commit proecure and
//     /// return commitments
//     End {
//         query_id: indexer::QueryId,
//         sender: oneshot::Sender<
//             Result<
//                 (
//                     // commitments sent
//                     Vec<Commit>,
//                     // commitments received
//                     Vec<Commit>,
//                 ),
//                 Box<anyhow::Error>,
//             >,
//         >,
//     },
//     /// Process requst received over
//     /// network for query
//     ReceivedRequest {
//         peer_id: PeerId,
//         request_id: request_response::RequestId,
//         request: network::CommitRequest,
//     },
// }

// #[derive(Default)]
// struct PeerWallet {
//     pub wallet_address: Address,
//     pub owner_address: Address,
// }

// struct Procedure {
//     request: Request,
//     peer_wallet: PeerWallet,
//     commitments_sent: HashMap<u32, Commit>,
//     commitments_received: HashMap<u32, Commit>,
//     network_client: network::Client,
//     subscription_interface: subscription::Subscription,
//     subscription_receiver: mpsc::Receiver<subscription::ResponseWrapper>,
//     pending_susbscription_requests: HashMap<usize, subscription::Request>,
//     command_receiver: mpsc::Receiver<ProcedureCommand>,
//     request_sender: mpsc::Sender<ProcedureRequest>,
//     node: EthNode,
// }

// #[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
// enum RoundType {
//     /// Commitment Type 1
//     T1 = 1,
//     /// Commitment Type 2
//     T2 = 2,
// }

// impl Procedure {
//     pub fn new(
//         request: Request,
//         network_client: network::Client,
//         command_receiver: mpsc::Receiver<ProcedureCommand>,
//         request_sender: mpsc::Sender<ProcedureRequest>,
//         node: EthNode,
//     ) -> Self {
//         let (subscription_receiver, subscription_interface) =
//             subscription::new(network_client.clone());

//         Self {
//             request,
//             peer_wallet: Default::default(),
//             commitments_sent: Default::default(),
//             commitments_received: Default::default(),
//             subscription_interface,
//             subscription_receiver,
//             pending_susbscription_requests: Default::default(),
//             network_client,
//             command_receiver,
//             request_sender,
//             node,
//         }
//     }

//     pub async fn start(mut self) -> Result<(), anyhow::Error> {
//         debug!(
//             "(commit procedure) procedure start for query_id {:?}",
//             self.request.query_id()
//         );
//         let mut interval = time::interval(time::Duration::from_millis(100));
//         loop {
//             select! {
//                 command = self.command_receiver.recv() => {
//                     if let Some(c) = command {
//                         match c {
//                             ProcedureCommand::ReceivedRequest {
//                                 peer_id,
//                                 request_id,
//                                 request,
//                             } => {
//                                 match request {
//                                     network::CommitRequest::CommitFund {
//                                         query_id,
//                                         round,
//                                     } => {
//                                         if query_id == self.request.query_id() && self.is_valid_round_request(round) {
//                                             match self.find_commitment_for_round(round, self.round_commitment_type(round, self.request.is_requester)).await {
//                                                 Ok(commitment) => {
//                                                     // send response to the request
//                                                     match self.network_client.send_dse_message_response(request_id, network::DseMessageResponse::Commit(
//                                                         network::CommitResponse::CommitFund {
//                                                             query_id,
//                                                             round,
//                                                             commitment:commitment.clone(),
//                                                         }
//                                                     )).await {
//                                                         Ok(_) => {
//                                                             debug!("(commit procedure) DSE CommitFund response {:?} for round {:?} for query_id {:?} sent", commitment.clone(), round, query_id);
//                                                         },

//                                                         Err(e)=> {
//                                                             error!("(commit procedure) DSE CommitFund response {:?} for round {:?} for query_id {:?} failed with error: {:?}", commitment.clone(), round, query_id, e);
//                                                         }
//                                                     }
//                                                 },
//                                                 Err(_) => {
//                                                    error!("(commit procedure) unable to find commitment for round {:?} for query_id {:?}",  round, query_id);
//                                                 }
//                                             }
//                                         }
//                                     },
//                                     network::CommitRequest::InvalidatingSignature {
//                                         query_id,
//                                         invalidating_signature,
//                                     } => {

//                                     },
//                                     network::CommitRequest::WalletAddress (query_id) => {
//                                         debug!("(commit procedure) DSE WalletAddres request received from peer id {:?} for query id {:?}" ,peer_id, query_id);
//                                         if peer_id == self.request.counter_party_peer_id() {
//                                             match  self.network_client.send_dse_message_response(request_id, network::DseMessageResponse::Commit(network::CommitResponse::WalletAddress {
//                                             query_id,
//                                             wallet_address: self.node.timelocked_wallet
//                                         })).await {
//                                             Ok(_) => {
//                                                 debug!("(commit procedure) DSE WalletAddress response sent to peer id {:?} for query id {:?}", peer_id, query_id);
//                                             },
//                                             Err(e) => {
//                                                 error!("(commit procedure) DSE WalletAddress response to peer id {:?} for query id {:?} failed with error: {:?}",  peer_id, query_id, e);
//                                             }
//                                         }
//                                         }else {
//                                             error!("(commit procedure) DSE WalletAddress for query id {:?} was received from peer id {:?} but countery party peer id is {:?}", query_id,peer_id ,self.request.counter_party_peer_id())
//                                         }
//                                     }
//                                     _ => {}
//                                 }
//                             },
//                             ProcedureCommand::End {
//                                 query_id,
//                                 sender
//                             } => {
//                                 if query_id == self.request.query_id() {
//                                     debug!("(commit procedure) end command received for query {:?}", query_id.clone());
//                                     let _ = sender.send(Ok((
//                                         self.commitments_sent.clone().into_values().collect(),
//                                         self.commitments_received.clone().into_values().collect(),
//                                     )));

//                                     // END procedure
//                                     return Ok(());

//                                 }else {
//                                     let _ = sender.send(Err(Box::new(anyhow::anyhow!("Wrong query id!"))));
//                                 }
//                             },
//                             _ => {}
//                         }
//                     }
//                 },
//                 Some(response) = self.subscription_receiver.recv() => {
//                      if let Some(pending_request) = self.pending_susbscription_requests.remove(&response.id)  {
//                         match response.response {
//                             subscription::Response::DseRequest(network::DseMessageResponse::Commit(network::CommitResponse::CommitFund {
//                                 query_id,
//                                 round,
//                                 commitment
//                             })) =>{
//                                 if query_id == self.request.query_id() {

//                                     // TODO check validity on chain and in DHT

//                                     // TODO check index is valid according to peer's wallet

//                                     // TODO check epoch is valid according to peer's wallet (probably also check that there
//                                     // exists enough time before epoch expires)

//                                     // if
//                                     // // we are checking type of commitment expected
//                                     // // from the peer for this round, thus we negate is_requester
//                                     // commitment.c_type == self.round_commitment_type(round, !self.request.is_requester) &&
//                                     // // u should match query_id
//                                     // commitment.u == self.request.query_id() &&
//                                     // // if self is requester, then i_address should be of self
//                                     // ((self.request.is_requester && commitment.i_address == self.node.signer_address()) ||
//                                     // // if self is not requester, then i_address should be of peer
//                                     // (!self.request.is_requester && commitment.i_address == self.peer_wallet.owner_address)) &&
//                                     // // If commit is of type 2, then r_address should of self.
//                                     // (commitment.c_type == RoundType::T2 && commitment.r_address == self.node.signer_address() ||commitment.c_type == RoundType::T1)
//                                     // {
//                                     //     // commitment received
//                                     //     debug!("(commit procedure) received commit for query_id {:?} for round {:?}", query_id.clone(), round.clone());
//                                     //     self.commitments_received.insert(round, commitment.clone());
//                                     // } else {
//                                     //     error!("(commit procedure) received invalid commit for query_id {:?} for round {:?}", query_id.clone(), round.clone());

//                                     // }
//                                     // commitment received
//                                     debug!("(commit procedure) received commit for query_id {:?} for round {:?}", query_id.clone(), round.clone());
//                                     self.commitments_received.insert(round, commitment.clone());
//                                 }

//                             },
//                             subscription::Response::DseRequest(network::DseMessageResponse::Commit(network::CommitResponse::WalletAddress {
//                                 query_id,
//                                 wallet_address
//                             })) => {
//                                 if query_id == self.request.query_id() {
//                                     debug!("(commit procedure) received peer wallet_address {:?} for query_id {:?}", wallet_address.clone(), query_id.clone());
//                                     self.peer_wallet = PeerWallet {
//                                         wallet_address,
//                                         // TODO query this from on-chain
//                                         owner_address: Address::random(),
//                                     }
//                                 } else {
//                                     error!("(commit procedure) received peer wallet_address {:?} for query_id {:?}, but was requested for query id {:?}", wallet_address.clone(), query_id.clone(), self.request.query_id());
//                                 }
//                             }
//                             subscription::Response::Error(e) => {
//                                 error!("(commit procedure) pending subscription request {:?} failed", pending_request);
//                             }
//                             _ => {}
//                         }
//                     }else {
//                         error!("(commit procedure) unable to find pending subscription with id {}", response.id);
//                     }
//                 }
//                 _ = interval.tick() => {
//                     debug!("Hello");
//                     self.handle().await;
//                 }
//             }
//         }
//     }

//     /// Returns true if counter party has asked for
//     /// commitment of a valid round.
//     /// A provider only commits in odd rounds, whereas
//     /// request commits in all rounds.
//     /// Commit request for a round is invalid if
//     /// round > expected round
//     fn is_valid_round_request(&self, r_round: u32) -> bool {
//         // provider does not responds to even rounds
//         if self.request.is_requester == false && r_round % 2 == 0 {
//             return false;
//         }

//         if r_round > self.next_expected_round() {
//             return false;
//         }

//         true
//     }

//     async fn handle(&mut self) {
//         debug!(
//             "(commit procedure) time to process commit for query_id {:?}",
//             self.request.query_id()
//         );

//         // avoid sending DSE request if there's already a pending
//         // subscription for one (atleast for now)
//         if self.pending_susbscription_requests.is_empty() {
//             if self.peer_wallet.wallet_address != Address::default()
//                 && self.peer_wallet.owner_address != Address::default()
//             {
//                 // figure out the index to ask for (i.e. round)
//                 let round = self.next_expected_round();

//                 error!("THis is the round {}", round);

//                 if round == self.rounds() {
//                     // All necessary commitments have been received,
//                     // thus notify main.
//                     // Note that we can't end thread rn, since the
//                     // counter party might ask for pending commits from
//                     // self.
//                     // TODO: notify main to wait for service
//                     debug!(
//                         "(commit procedure) all commits have been received for query_id {:?}",
//                         self.request.query_id()
//                     );
//                 } else {
//                     // send round request
//                     debug!("(commit procedure) Sending DSE CommitFund request to peer id {:?} for query id {:?} for round {:?}", self.request.counter_party_peer_id(),self.request.query_id(), round);
//                     let subscription_req = subscription::Request::DseRequest {
//                         peer_id: self.request.counter_party_peer_id(),
//                         message: network::DseMessageRequest::Commit(
//                             network::CommitRequest::CommitFund {
//                                 query_id: self.request.query_id(),
//                                 round,
//                             },
//                         ),
//                     };
//                     let id = self.subscription_interface.start(subscription_req.clone());
//                     self.pending_susbscription_requests
//                         .insert(id, subscription_req);
//                 }
//             } else {
//                 // ask for counter wallet address
//                 debug!("(commit procedure) Sending DSE WalletAddress request to peer id {:?} for query id {:?}", self.request.counter_party_peer_id(),self.request.query_id());
//                 let subscription_req = subscription::Request::DseRequest {
//                     peer_id: self.request.counter_party_peer_id(),
//                     message: network::DseMessageRequest::Commit(
//                         network::CommitRequest::WalletAddress(self.request.query_id()),
//                     ),
//                 };
//                 let id = self.subscription_interface.start(subscription_req.clone());
//                 self.pending_susbscription_requests
//                     .insert(id, subscription_req);
//             }
//         } else {
//             debug!("(commit procedure) Pending subcription for DSE message request");
//         }
//     }

//     /// Lastest round for which commitment
//     /// is pending from the counter party
//     fn next_expected_round(&self) -> u32 {
//         for n in 0..self.rounds() {
//             // provider only sends commitments in odd rounds
//             // since it only has to commit half as much as requester
//             if ((self.request.is_requester && n % 2 == 1) || !self.request.is_requester)
//                 && !self.commitments_received.contains_key(&n)
//             {
//                 return n;
//             }
//         }
//         // recieved all commitments
//         self.rounds()
//     }

//     /// Returns round commitment type (either T1 or T2).
//     /// All commitments by service provider are of T1, whereas
//     /// first 50% (i.e. commitments in round < total_rounds / 2)
//     /// commitments from requester are of T1 and last 50%
//     /// are of T2.
//     fn round_commitment_type(&self, round: u32, is_requester: bool) -> RoundType {
//         // requester's commitments are of type 2 after round/2
//         if is_requester && round >= (self.rounds() / 2) {
//             return RoundType::T2;
//         }

//         RoundType::T1
//     }

//     /// Returns total number of commitment rounds
//     /// required for this exchange.
//     /// total_rounds = bid.charge * 2. This is
//     /// because amount that is to be committed by
//     /// requester is bid.charge * 2.
//     /// provider only commits amount = bid.charge.
//     fn rounds(&self) -> u32 {
//         // TODO: rn we have hardcoded that the per index value
//         // is 1*10**16 (i.e. cent). Make it configurable.
//         // Amount to be committed by requester = charge * 2
//         // Amount to be committed by provider = charge
//         ((self.request.bid.bid.charge * U256::from_str("2").unwrap())
//             / U256::from_dec_str("10000000000000000").unwrap())
//         .as_u32()
//     }

//     // Returns commitment of the node
//     // for a given round.
//     async fn find_commitment_for_round(
//         &mut self,
//         round: u32,
//         c_type: RoundType,
//     ) -> Result<Commit, anyhow::Error> {
//         // look whether there already exists
//         // commitment for the round
//         match self.commitments_sent.get(&round) {
//             Some(val) => Ok(val.clone()),
//             None => {
//                 let (sender, receiver) = oneshot::channel::<Result<u32, anyhow::Error>>();
//                 self.request_sender
//                     .send(ProcedureRequest::FindCommitmentIndex {
//                         query_id: self.request.query_id(),
//                         sender,
//                     })
//                     .await?;
//                 let index = receiver.await??;
//                 // create commitment by c_type for the index
//                 let mut commit = Commit {
//                     index,
//                     epoch: 0,
//                     u: self.request.query_id(),
//                     c_type: c_type.clone(),
//                     i_address: if self.request.is_requester {
//                         self.node.signer_address()
//                     } else {
//                         self.peer_wallet.owner_address
//                     },
//                     r_address: match c_type {
//                         RoundType::T2 => self.peer_wallet.owner_address,
//                         _ => Address::zero(),
//                     },
//                     signature: None,
//                     invalidating_signature: None,
//                 };
//                 commit.signature = Some(self.node.sign_message(commit.commit_hash()));
//                 self.commitments_sent.insert(round, commit.clone());
//                 Ok(commit)
//             }
//         }
//     }
// }

// #[derive(Debug)]
// pub enum Command {
//     /// Process new request in new handler
//     StartCommitProcedure {
//         request: Request,
//         sender: oneshot::Sender<Result<(), anyhow::Error>>,
//     },
//     /// Received a request over network
//     ReceivedRequest {
//         peer_id: PeerId,
//         request: network::CommitRequest,
//         request_id: request_response::RequestId,
//         sender: oneshot::Sender<Result<(), anyhow::Error>>,
//     },
//     /// Produces invalidting signature (used by service requester)
//     ProduceInvalidatingSignature {
//         query_id: indexer::QueryId,
//         sender: oneshot::Sender<Result<(), anyhow::Error>>,
//     },
//     /// End commit procedure for a query id.
//     /// Used for ending commit procedure by force
//     ForceEndCommit {
//         query_id: indexer::QueryId,
//         sender: oneshot::Sender<Result<(), anyhow::Error>>,
//     },
// }

// #[derive(Clone)]
// pub struct Client {
//     pub command_sender: mpsc::Sender<Command>,
// }

// impl Client {
//     pub async fn start_commit_procedure(&mut self, request: Request) -> Result<(), anyhow::Error> {
//         let (sender, receiver) = oneshot::channel();
//         self.command_sender
//             .send(Command::StartCommitProcedure { request, sender })
//             .await
//             .expect("Command message dropped");
//         receiver.await.expect("Client response dropped")
//     }

//     pub async fn handle_received_request(
//         &mut self,
//         peer_id: PeerId,
//         request: network::CommitRequest,
//         request_id: request_response::RequestId,
//     ) -> Result<(), anyhow::Error> {
//         let (sender, receiver) = oneshot::channel();
//         self.command_sender
//             .send(Command::ReceivedRequest {
//                 peer_id,
//                 request,
//                 request_id,
//                 sender,
//             })
//             .await
//             .expect("Command message dropped");
//         receiver.await.expect("Client response dropped")
//     }

//     pub async fn produce_invalidating_signature(
//         &mut self,
//         query_id: indexer::QueryId,
//     ) -> Result<(), anyhow::Error> {
//         let (sender, receiver) = oneshot::channel();
//         self.command_sender
//             .send(Command::ProduceInvalidatingSignature { query_id, sender })
//             .await
//             .expect("Command message dropped");
//         receiver.await.expect("Client response dropped")
//     }

//     pub async fn force_end_commit_procedure(
//         &mut self,
//         query_id: indexer::QueryId,
//     ) -> Result<(), anyhow::Error> {
//         let (sender, receiver) = oneshot::channel();
//         self.command_sender
//             .send(Command::ForceEndCommit { query_id, sender })
//             .await
//             .expect("Command message dropped");
//         receiver.await.expect("Client response dropped")
//     }
// }

// pub struct Commitment {
//     /// Receives commands
//     command_receiver: mpsc::Receiver<Command>,
//     /// Network client
//     network_client: network::Client,
//     /// Used for handlers to send events
//     procedure_request_sender: mpsc::Sender<ProcedureRequest>,
//     /// Receives events from all handlers
//     procedure_request_receiver: mpsc::Receiver<ProcedureRequest>,
//     /// Requests under process
//     ongoing_procedures: HashMap<indexer::QueryId, (Request, mpsc::Sender<ProcedureCommand>)>,
//     /// Indexes not used for commitment
//     unused_indexes: Vec<u32>,
//     /// Indexes spent on T2 commits
//     type2_spent_commits: HashMap<indexer::QueryId, Vec<Commit>>,
//     /// T1 commitments received
//     type1_recv_commitments: HashMap<indexer::QueryId, Vec<Commit>>,
//     /// T2 commitments received
//     type2_recv_commitments: HashMap<indexer::QueryId, Vec<Commit>>,
//     /// Invalidating signatures by query id
//     invalidating_signatures: HashMap<indexer::QueryId, Signature>,
//     /// eth node instance
//     node: EthNode,
// }

// impl Commitment {
//     pub fn new(
//         command_receiver: mpsc::Receiver<Command>,
//         network_client: network::Client,
//         node: EthNode,
//     ) -> Self {
//         let (procedure_request_sender, procedure_request_receiver) = mpsc::channel(10);

//         // FIX: dummy temp  indexes
//         let mut unused_indexes = Vec::<u32>::new();
//         for v in 1..100 {
//             unused_indexes.push(v);
//         }

//         Self {
//             command_receiver,
//             network_client,
//             procedure_request_sender,
//             procedure_request_receiver,
//             ongoing_procedures: Default::default(),
//             unused_indexes: unused_indexes,
//             type2_spent_commits: Default::default(),
//             type1_recv_commitments: Default::default(),
//             type2_recv_commitments: Default::default(),
//             invalidating_signatures: Default::default(),
//             node,
//         }
//     }

//     pub async fn run(mut self) {
//         // Every 20 seconds or so check which commit procedures should be ended
//         let mut interval = time::interval(time::Duration::from_secs(20));

//         loop {
//             select! {
//                 command = self.command_receiver.recv() => {
//                     match command {
//                         Some(c) => {self.command_handler(c).await},
//                         None => {}
//                     }
//                 },
//                 event = self.procedure_request_receiver.recv() => {
//                     match event {
//                         Some(e) => {self.procedure_requests(e).await},
//                         None => {}
//                     }
//                 },
//                 _ = interval.tick() => {
//                     debug!("(commitment) time to close commit procedures");

//                     let mut remove_set: HashSet<indexer::QueryId> = HashSet::new();
//                     for (query_id, (request, m_sender)) in self.ongoing_procedures.iter() {
//                         // check invalidating signature is present or not
//                         if self.invalidating_signatures.contains_key(&query_id) {
//                             // end commit procedure
//                             let (sender, receiver) = oneshot::channel();
//                             m_sender
//                                 .send(ProcedureCommand::End {
//                                     query_id: query_id.clone(),
//                                     sender,
//                                 })
//                                 .await;
//                             match receiver
//                                 .await
//                                 .expect("commitment: Failed to end commit procedure")
//                             {
//                                 Ok(res) => {
//                                     // process sent commits
//                                     let mut t2_spent_commits: Vec<Commit> = Vec::new();
//                                     for commit in res.0.iter() {
//                                         if commit.c_type == RoundType::T1 {
//                                             // t1 indexes are free to be used again
//                                             self.unused_indexes.push(commit.index);
//                                         } else {
//                                             t2_spent_commits.push(commit.clone());
//                                         }
//                                     }
//                                     // t2 commits (& indexes) are spent unless not redeemed before
//                                     // next epoch
//                                     self.type2_spent_commits
//                                         .insert(query_id.clone(), t2_spent_commits);

//                                     // process received commits
//                                     let mut t1_recv_commits: Vec<Commit> = Vec::new();
//                                     let mut t2_recv_commits: Vec<Commit> = Vec::new();
//                                     for commit in res.1.iter() {
//                                         if commit.c_type == RoundType::T1 {
//                                             // t1 indexes are free to be used again
//                                             t1_recv_commits.push(commit.clone());
//                                         } else {
//                                             t2_recv_commits.push(commit.clone());
//                                         }
//                                     }
//                                     self.type1_recv_commitments
//                                         .insert(query_id.clone(), t1_recv_commits);
//                                     self.type2_recv_commitments
//                                         .insert(query_id.clone(), t2_recv_commits);

//                                     remove_set.insert(query_id.clone());
//                                 }
//                                 Err(_) => {
//                                     // TODO handle error here
//                                 }
//                             }
//                         }
//                     }
//                     // remove ongoing_procedures in remove_set
//                     self.ongoing_procedures
//                         .retain(|&k, _| !remove_set.contains(&k));
//                 }
//             }
//         }
//     }

//     pub async fn command_handler(&mut self, command: Command) {
//         match command {
//             Command::ReceivedRequest {
//                 peer_id,
//                 request_id,
//                 request,
//                 sender,
//             } => {
//                 match request {
//                     // Recevied invalidting signature from service requester
//                     network::CommitRequest::InvalidatingSignature {
//                         query_id,
//                         invalidating_signature,
//                     } => {
//                         match self.ongoing_procedures.get(&query_id) {
//                             Some((request, sender)) => {
//                                 // peer can produce and send invalidating signature
//                                 // if self is not requester
//                                 if request.is_requester == false {
//                                     // TODO check invalidating signatur is valid

//                                     self.invalidating_signatures
//                                         .insert(query_id, invalidating_signature);

//                                     self.network_client
//                                         .send_dse_message_response(
//                                             request_id,
//                                             network::DseMessageResponse::Commit(
//                                                 network::CommitResponse::AckInvalidatingSignature(
//                                                     query_id,
//                                                 ),
//                                             ),
//                                         )
//                                         .await;
//                                 }
//                             }
//                             None => {}
//                         }
//                     }
//                     network::CommitRequest::CommitFund { query_id, round } => {
//                         match self.ongoing_procedures.get(&query_id) {
//                             Some((_, sender)) => {
//                                 sender
//                                     .send(ProcedureCommand::ReceivedRequest {
//                                         peer_id,
//                                         request_id,
//                                         request,
//                                     })
//                                     .await;
//                             }
//                             None => {}
//                         }
//                     }
//                     network::CommitRequest::WalletAddress(query_id) => {
//                         match self.ongoing_procedures.get(&query_id) {
//                             Some((_, sender)) => {
//                                 let res = sender
//                                     .send(ProcedureCommand::ReceivedRequest {
//                                         peer_id,
//                                         request_id,
//                                         request,
//                                     })
//                                     .await;
//                             }
//                             None => {}
//                         }
//                     }
//                     _ => {}
//                 }

//                 sender.send(Ok(()));
//             }
//             Command::StartCommitProcedure { request, sender } => {
//                 // create new handler
//                 let (p_sender, p_receiver) = mpsc::channel::<ProcedureCommand>(10);
//                 // insert sender for procedure to pending requests
//                 self.ongoing_procedures
//                     .insert(request.query_id().clone(), (request.clone(), p_sender));
//                 let procedure = Procedure::new(
//                     request,
//                     self.network_client.clone(),
//                     p_receiver,
//                     self.procedure_request_sender.clone(),
//                     self.node.clone(),
//                 );
//                 tokio::spawn(procedure.start());

//                 sender.send(Ok(()));
//             }
//             Command::ProduceInvalidatingSignature { query_id, sender } => {
//                 match self.ongoing_procedures.get(&query_id) {
//                     Some((request, _)) => {
//                         // can produce invalidating singature only when self is requester
//                         if request.is_requester {
//                             let mut q_id: [u8; 256] = [0; 256];
//                             U256::from(query_id).to_little_endian(&mut q_id);
//                             let invalidating_signature =
//                                 self.node.sign_message(keccak256(q_id.as_slice()).into());
//                             self.invalidating_signatures
//                                 .insert(query_id, invalidating_signature);
//                         }
//                     }
//                     None => {}
//                 }
//                 sender.send(Ok(()));
//             }
//             Command::ForceEndCommit { query_id, sender } => {
//                 // TODO handle this
//                 sender.send(Ok(()));
//             }
//             _ => {}
//         }
//     }

//     pub async fn procedure_requests(&mut self, event: ProcedureRequest) {
//         match event {
//             ProcedureRequest::FindCommitmentIndex { query_id, sender } => {
//                 match self.find_index(query_id) {
//                     Some(v) => sender.send(Ok(v)),
//                     None => sender.send(Err(anyhow::anyhow!("Commitments: Err - no index left"))),
//                 };
//             }
//         }
//     }

//     pub fn find_index(&mut self, query_id: indexer::QueryId) -> Option<u32> {
//         self.unused_indexes.pop()
//     }

//     // // range usage is disabled for now
//     // pub fn find_index_range(&mut self, amount: U256) -> Option<(u32, u32)> {
//     //     let start: u32;
//     //     let end: u32;
//     //     for i in self.unused_indexes.iter() {
//     //         if (U256::from(end - start) * self.index_value) == amount {

//     //             return Some((start, end));
//     //         }
//     //         if end == i - 1 {
//     //             end += 1;
//     //         } else {
//     //             start = *i;
//     //             end = *i;
//     //         }
//     //     };
//     //     None
//     // }
// }

// pub fn new(
//     command_receiver: mpsc::Receiver<Command>,
//     network_client: network::Client,
//     node: EthNode,
// ) -> Commitment {
//     Commitment::new(command_receiver, network_client, node)
// }

// // Progrssive commitment
// // 1
// // 1    1
// // 1
// // 1    1
// // 1
// // 1    1
// // 1
// // 1    1
// // 1
// // 1    1

// // issues
// // How to handle commitment of type 1 and 2
// // How to make sure handle commitments are trsmitted to global storag

// // Figure out whether handler or commitment will take care of commitment signature
// // Handler needs to validate & sign

// // TODO
// // 1. Add fns for creating signatures and verification

// // Command to be used by service requester to produce
// // invalidating signatures for the query
// // Command::ProduceInvalidatingSignature(query_id) => {
// //     // Only service requester can produce invalidating signature
// //     if query_id == self.request.query_id() && self.request.is_requester == true {
// //         // TODO produce invalidating signature & send it country party
// //         let invalidating_signature: String = "FakeSiganture".into();

// //         self.handler_sender.send(HandlerEvent::AddInvalidatingSignature {
// //             query_id,
// //             signature: invalidating_signature.clone(),
// //         }).await;

// //         match self.network_client.send_dse_message_request(self.request.counter_party_peer_id(), network::DseMessageRequest::Commit(
// //             network::CommitRequest::InvalidatingSignature {
// //                 query_id,
// //                 invalidating_signature,
// //             }

// //             // TODO END the handler
// //         )).await {
// //             Ok(res) => {},
// //             Err(_) => {
// //                 // TODO probably try again?
// //             }
// //         }
// //     }
// // }
