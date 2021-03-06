# DSE

[IN PROGRESS]

How about you can trade information/service in exchange of a fee with anyone on interent without having to trust/knowing them? Wouldn't it be amazing if you can just shoot a query for some information on a network and get a worthwile response? You can ask for your anynomity score, your digital traces, data-sets for training, high quality data, code, or anything you can imagine!

This is what DSE aims to acheive.

DSE (Decentralised Search Engine) is a decentralised p2p marketplace for information/service. Where one that is willing to pay for a query can find and trade with the one having the capacity of fulfill the query in exchange of fee, without trusting/knowing one another.

To make sure every exchange is trustless, DSE uses 2 of 2 scorched earth mechanism. In which, for exchange, interested parties temporarily lock up funds of greater value than the exchange itself. If exchange goes well funds are unlocked, otherwise funds stay locked up forever. This way it is made sure that the requester (one with query) and provider (one with capacity to fullfil) have more to lose if exchange does not goes well. Since it is "requester" that decides whether the provided service was good and unlock locked funds, "provider" is forced to provide high quality service. Moreover, as even "requester" has value to lose if they don't unlock funds, they are expected to not unlock only in cases where provided service was really bad.

To enable an active p2p marketplace that allows for providers to service queries of as cheap as few cents with high frequency, exchanges happen off-chain. This is done by using time-locked commitments (backed by time-locked wallets on-chain) + state channels as means of payments.

Services that you can imagine being offered on DSE

-   Providing high quality data in a specific domain
-   Vertical search engines
-   Factually correct news aggregation
-   Dataseet for training purposes
-   Services for different gigs (code, data collection & labelling, articles, etc.)
-   Data inference on well trained models
-   Service for finding your anonymity score on internet
-   p2p exchange of goods

<!-- How about the ppl that work for optimsing Ads at google & fb could spend sometime build a indexer in a specialised domain and put it on service on DSE?

They can potentially earn good amount of money. This does not have to be full-time gig, but sort of a fun gig. -->

## Spec

DSE p2p network uses -

-   Gossipsub pub-sub protocol for propogating new search queries in the network.
-   Kademlia DHT for peer discovery and indexing of time-locked commitments.
-   Request-Response protocol for placing bids, commitments, trades, and other necessary messages.

DSE uses time-locked commitments and state channels for off-chain exchanges.

### Peer discovery

### Information Query and Bids

Consider service requester node **A** and service provider node **B**.

1. Node A publishes their "Query" on the p2p network. Every node on the network subscribes to "Query" topics, thus A's query is progated throughout the network.
2. Node B receives Node A's query and processes. After processing it decides to place a bid.
3. B sends "PlaceBid" request to A, after which A sends "AckPlaceBid" response to B. (A would have received "PlaceBid" request from several nodes)
4. A select B's bid as the winning bid. A sends "AcceptBid" request to B, after which B sends "AckAcceptBid" response to B.
5. B checks whether still want to seervice A's query. If yes, B proceeds to (6). If no, interaction stops here and A would choose some other bid after a buffer period.
6. B sends "StartCommit" request to A, after which A sends "AckStartCommit" response to B.
7. Commitment of funds necessary for the exchange happens. Once commitment of funds end, A & B "FundsLocked" event.
8. B sends "QueryResponse" request to A, after which A sends "AckQueryResponse" response to B.
9. [WIP...]

### Time locked wallets & commitments

Time-locked wallets work the following way -

1. User deposits X amount to time-locked wallet. This funds the wallet by Y amount (Y = X/2) and rest of the amount is considered security deposit.
2. Time-locked wallets have a definite epoch period (that can range from a week to months).
3. Y amount is divided into 1 cents with every cent having a corresponding index (starting from 0).
4. User can commit a specific index range for specific epoch to someone else as a means of payment.
5. Users that receive Time-locked commitments can redeem equivalent amount ((1 cent \* index range) per commitment) from the time-locked wallet before the current epoch expires.
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

To minize the loss when one party renege from locking up funds for commitment, whereas other has already committed, commitment is made progressively.

So if Node A and B agreed to lock amount X for the trade, they progressively commit 0.1X in 10 rounds. Where every subsequent round of commitment only happends if both Node A and B committed their rspective share in last round. Thus, the malcious user can only fraud honest user for 0.1X.

### Commitment types

Type 1:
**Sign(a - b, e, u, 1, address)**, where a - b is index range, e is epoch number, u is unique id, 1 is type, and address is of address of service requester.
Funds committed using type 1 will go to address 0 (burned), unless invalidating signature is produced.
Invalidating signature is - Sign(u) by address. This invalidates the commitment and the indexes can be used for some other commitment. Type 1 commitments are used fo locking funds for the trade.

Type 2:
**Sign(a - b, e, u, 2, address, recv_address)** where a - b is index range, e is epoch number, u is unique id, 2 is type, and address is of address of service requester.
Funds committed using type 2 will go to address 0 (burned), unless invalidating signature is produced. Invalidating signature - Sign(u), redirects the funds to recv_address. Type 2 commitments are used for payments.

### Commitment of Funds

Consider node A (request node) and node B (provider node):
Node B charges 0.5X, therefore Node A commits X amount and Node B commits 0.5X.
The commitment happens over 10 rounds, that means in every round both node A and B commit 0.1X.
Commitment happens the following way -

1. A commits 0.1X as commitment type 1 for first 5 rounds. After which, for the next 5 rounds, A commits 0.1X as commitment type 2.
2. B commits 0.05X as commitment type 1 for all 10 rounds.
3. At every commitment round, A or B won't proceed to next round unless they have received??& verified validity of expected commitment from the counterparty for the current round.
4. After receiving a commitment the nodes does the following to verify commitment's validity -
   For every index in commitment
    - Node checks whether index has been committed to some other address by sending DHT get query on the network.
    - DHT query is expected to return a list of addresses (multiaddresses) that claim that the index was committed to them before.
    - If DHT returns an empty list, node assumes index in the commitment is valid.
    - If commitment is type 2 and DHT returns a list greater than length 0, node rejects the commitment.
    - If commitment is type 1 and DHT returns a list greater than length 0, then node sends a "ProvideCommitment" request to all addresses.
    - The addresses either respond with their respective commitment OR commitment + invalidating signature.
    - For every commitment received from addresses, node verifies their correctness. Checks whether they are type 1 or 2. If any of them is type 2, node forwards it to all addresses in the list and rejects the initial commitment. Otherwise, node checks whether they have respective invalidating signature and proceeds further.
    - For the commitments that do not have respective invalidating signatures, node asks counteryparty for invalidating signature.
    - If counterparty isn't able to provide invalidating signature, node rejects the initial commitment.
    - If counteryparty provides a invalidating signature, node sends it to the addresses returned in DHT get query.
    - Node proceeds to check next index.

<!-- ### Claiming commitments on-chain -->

<!-- ### Extras

1. Unique ID for a search query - Keccack256(Peer_id + Query count)  -->
<!--
### Random questions

1. Why do we need gasless trades?
   Having gasless trades allows for service providers to serve queries at higher frequency. Even when on-chain txs costs are 1 cent/tx, if every trade requires on-chain interaction service providers will ultimately be limited by tx cost.
 -->
