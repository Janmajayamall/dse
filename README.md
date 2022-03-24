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

### Lifecycle of a trade

Consider requester node as **A** and provider node as **B**.

1. Node A publishes a information query on the p2p network. Since every node understands gossipsub protocol and is subscribed to "Query" topic, the query will be relayed to every node.
2. Consider that node B, B1, B2, B3 are interested in servicing the query. All of the will prepare their **Bid** for servicing the query and send it to Node A. Node A upon receiving their bid will reply with acknowledgement.
3. Node A aftere reviewing all bids, selects node B's bid. Node A sends **Bid Acceptance** to Node B. Node B upon receving bid acceptance replies with acknoledgement.
4. Node A and B now engage in commitment of funds (as specified in the bid) for the trade. Funds commitement follow **Progressive commitment**.
5. Node B now proceeds to service the query of Node A
6. [Pending]

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

Used for making redeeming commitments and proving double commitments on-chain cheap.

### Progressive commitment

Progressive commitment tackles the following situation -
Node A (service requester) and Node B (service provider) wants to trade, but Node A is malicious. Node A accepts Node B's bid. After which Node B proceeded to lock up funds necessary for the trade. But Node A reneges and does not commit funds. This results in situation where Node B has unecesssarily locked up some amount for the current epoch (in case of time-locked commitments). Even worse, Node A can also post the commitment on chain to burn amount equivalent to Node B's commitment.

To minize the loss when one party renges from locking up funds for commitment, whereas other has already committed, commitment is made progressively.

So if Node A and B agreed to lock amount X for the trade, they progressively commit 0.1X in 10 rounds. Where every subsequent round of commitment only happends if both Node A and B committed their rspective share in last round. Thus, the malcious user can only fraud honest user for 0.1X.

### Random questions

1. Why do we need gasless trades?
   Having gasless trades allows for service providers to serve queries at higher frequency. Even when on-chain txs costs are 1 cent/tx, if every trade requires on-chain interaction service providers will ultimately be limited by tx cost.
