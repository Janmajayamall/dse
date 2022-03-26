# DSE

[DOCUMENT IS IN PROGRESS]

DSE (Decentralised Search Engine) is p2p marketplace for information queries. Using DSE, 2 parties where one has a search query and another has the capacity to serve that query can discover and trade reliably.

DSE's design aims to -

1. Make exchange between two interested parties trustless using 2 of 2 scorched earth mechanism.
2. Enable gasless trades, making trades of value as low as 1 cent possible.

Use of DSE isn't just limited to a marketplace of information queries. It can easily be extended to a general marketplace where 2 parties can trade of value as low as few cents in absence of a trusted 3rd party.

## Outline

To keep the outline general, I will refer to a "search query" as a "service request" and a response to the query as "service fullfilment".

DSE's p2p network requires a node to participate. Nodes in the network gossip about every new service request, so that the nodes that are interested in fulfilling the request are made aware of it. Nodes interested in fulfilling a requst, prepare and submit their bid to the requester of the service. The service requester selects and accepts one of the bid. After acceptance of the bid the service requester and provider proceed as follows -

Let's say X is the service requester and Y is service provider. X has accepted Y's bid for providing a service in exchange of 0.0001 ETH. X and Y will proceed to lock up 0.1 ETH as commitment to the trade. Once Y fulfills the service of X, X can unlock the commitment such that X gets 0.9999 back and Y gets 0.1001 back. X can also choose to not unlock the commitment, if it didn't like the service provided by Y. In this case, both X & Y would not get their commitment back.

By committing a value that is more than the value of the trade, X & Y have more to lose if the trade does not go right. This forces Y to provide a good quality service to X. And gives X the freedom to punish Y if the service wasn't good.

## P2P Marketplace for Information Queries

Why do we need it?

Since DSE is a p2p marketplace, I believe it will develop a competitve ecosystem of information query providers specialising in different domains. This will faciliatate high quality information retrieval, especially in domains that are hard to find on intrenet using regular search engines.

## Spec

DSE p2p network uses -

-   Gossipsub pub-sub protocol for propogating new search queries in the network.
-   Kademlia DHT for peer discovery and indexing of time-locked commitments.
-   Request-Response protocol for placing bids, commitments, trades, and other necessary messages.

DSE uses time-locked on-chain wallets and state channels for gasless trades, thus allowing trade of value as low as 1 cent to happen on p2p network.

### Peer discovery

### Information Query and Bids

Consider service requester node **A** and service provider node **B**.

1. Node A publishes their "Query" on the p2p network. Every node on the network subscribes to "Query" topics, thus A's query is progated throughout the network.
2. Node B receives Node A's query and processes. After processing it decides to place a bid.
3. B sends "PlaceBid" request to A, after which A sends "AckPlaceBid" response to B. (A would have received "PlaceBid" request from several nodes)
4. A select B's bid as the winning bid. A sends "AcceptBid" request to B, after which B sends "AckAcceptBid" response to B.
5. B checks whether still want to seervice A's query. If yes, B proceeds to (6). If no, interaction stops here and A would choose some other bid after a buffer period.
6. B sends "StartCommit" request to A, after which A sends "AckStartCommit" response to B.
7. Commitment of funds necessary for the exchange happens. Once commitment of funds end, A & B receive an event.
8. B sends "QueryResponse" request to A, after which A sends "AckQueryResponse" response to B.
9. [Unlocking of funds is still WIP...]

### Commitment of Funds

### Time locked wallets & commitments

Time-locked wallets work the following way -

1. User deposits X amount to time-locked wallet. This funds the wallet by Y amount (Y = X/2) and rest of the amount is considered security deposit.
2. Time-locked wallets have a definite epoch period (that can range from a week to months).
3. Y amount is divided into 1 cents with every cent having a corresponding index (starting from 0).
4. User can commit a specific index (i.e. 1 cent) for specific epoch to someone else as a means of payment.
5. Users that receive Time-locked commitments can redeem equivalent amount (1 cent per commitment) from the time-locked wallet before the current epoch expires.
6. Once epoch expires, all commitments for that epoch that haven't been redeemed expire as well. All indexes that haven't been used, or were used but were not redemeed, are transfered to the next epoch.
7. If there exist two conflicting commitments (i.e. commitment with same index and epoch made to two different addresses), the wallet deposit is slashed.

Using time-locked wallets a user can pay for trades (by sending commitments). Once a user has accumulated commitments from someone else that are more than a single tx fee, they can redeem the commitments.

With the use of time-locked commitments, trade can be made gassless. Thus, a user only has to pay for on-chain tx fee per user from whom they have received payments per epoch.

It's possible that two users only traded very few times during some epoch, such that tx cost for redeeming commitments before epoch expires is higher than commitments themselves. In such cases, the user that has to redeem commitments can either choose to redeem or to not redeem. If they think the payer didn't interact often because they didn't like the service, they can consider their service as "free trial". Otherwise, they can redeem and punish the payer.

### State channels

If 2 users interact very often, they can choose to setup a state channel between them and not use Time-locked wallets.

### BLS signatures

If we use BLS signature aggregation for proving commitment (in case of withdrawal or fraud) on-chain, it will require n + 1 pairing operations (n = no. of commitments, since every commitment is different).

Gas cost of a pairing operation (34000 _ n + 45000) is lot greater than gas cost of ECDSA verification (3000). But I think considering that on l2 - l2 gas price is around 0.001 and l1 data cost is around 40 (depends on l1 gas cost), BLS aggregation might still be a better option. This is because calldata in case of BLS aggregation would be just a signature (64 bytes) + list of indexes (32 bytes). In case of ECDSA calldata will be n _ 32 (n = no. of signatures) + list of indexes (32 bytes).

### Progressive commitment

Progressive commitment tackles the following situation -
Node A (service requester) and Node B (service provider) wants to trade, but Node A is malicious. Node A accepts Node B's bid. After which Node B proceeded to lock up funds necessary for the trade. But Node A reneges and does not commit funds. This results in situation where Node B has unecesssarily locked up some amount for the current epoch (in case of time-locked commitments). Even worse, Node A can also post the commitment on chain to burn amount equivalent to Node B's commitment.

To minize the loss when one party renges from locking up funds for commitment, whereas other has already committed, commitment is made progressively.

So if Node A and B agreed to lock amount X for the trade, they progressively commit 0.1X in 10 rounds. Where every subsequent round of commitment only happends if both Node A and B committed their rspective share in last round. Thus, the malcious user can only fraud honest user for 0.1X.

### Extras

[Will be organised into sections a bit later]

1. Unique ID for a search query - Keccack256(Peer_id + Query count)
2. Users will run their own node with which they will interact over a REST API (Will be developed later)
3.

### Random questions

1. Why do we need gasless trades?
   Having gasless trades allows for service providers to serve queries at higher frequency. Even when on-chain txs costs are 1 cent/tx, if every trade requires on-chain interaction service providers will ultimately be limited by tx cost.
