const { io } = require("socket.io-client");
const axios = require("axios");
const fetch = require("node-fetch");

const baseUrl = `http://127.0.0.1`;
const baseInstance = axios.create({
	baseUrl: baseUrl,
	timeout: 10000,
	headers: { "Content-Type": "application/json" },
	Proxy: undefined,
});

async function sendQuery(query, port) {
	const res = await fetch(`${baseUrl}:${port}/action`, {
		method: "post",
		body: JSON.stringify({ NewQuery: { query: { query: query } } }),
		headers: { "Content-Type": "application/json" },
	});
	console.log(await res);
}

async function sendBid(queryId, requesterId, port) {
	const res = await fetch(`${baseUrl}:${port}/action`, {
		method: "post",
		body: JSON.stringify({
			PlaceBid: {
				bid: {
					query_id: queryId,
					requester_id: requesterId,
					charge: "0x16345785D8A0000",
				},
			},
		}),
		headers: { "Content-Type": "application/json" },
	});
	console.log(await res.text());
}

async function acceptBid(queryId, bidderId, port) {
	const res = await fetch(`${baseUrl}:${port}/action`, {
		method: "post",
		body: JSON.stringify({
			AcceptBid: {
				query_id: queryId,
				bidder_id: bidderId,
			},
		}),
		headers: { "Content-Type": "application/json" },
	});
	console.log(await res.text());
}

async function startCommit(queryId, port) {
	const res = await fetch(`${baseUrl}:${port}/action`, {
		method: "post",
		body: JSON.stringify({
			StartCommit: {
				query_id: queryId,
			},
		}),
		headers: { "Content-Type": "application/json" },
	});
	console.log(await res.text());
}

async function receivedQueries(port) {
	const res = await fetch(`${baseUrl}:${port}/recvqueries`, {
		method: "get",
	});
	console.log(await res.json());
}

async function getQueryBids(queryId, port) {
	const res = await fetch(`${baseUrl}:${port}/querybids/${queryId}`, {
		method: "get",
	});
	console.log(await res.json());
}

(async () => {
	const queryId = 1;
	const requesterPort = 3000;
	const providerPort = 5000;
	const requesterPeerId =
		"16Uiu2HAm4ro25Yb85MzJcDdfTyks4NEgLSBTTnHs4UPcAmPg65ah";
	const providerPeerId =
		"16Uiu2HAmQAujJB3QQbD6TLax71DTSaHAmN1MdGbqPzPh6dsSj3Wh";

	const delay = 3;

	// requester sends query
	setTimeout(async () => {
		await sendQuery("yolo2", requesterPort);
	}, 0 * delay * 1000);

	// provider receieves query
	setTimeout(async () => {
		await receivedQueries(providerPort);
	}, 1 * delay * 1000);

	// provider places a bid to requester for query
	setTimeout(async () => {
		await sendBid(queryId, requesterPeerId, providerPort);
	}, 2 * delay * 1000);

	// requester receives bid
	setTimeout(async () => {
		await getQueryBids(1, requesterPort);
	}, 3 * delay * 1000);

	// requester accepts the bid by provider
	setTimeout(async () => {
		await acceptBid(1, providerPeerId, requesterPort);
	}, 4 * delay * 1000);

	// provider sends start commit to requester
	setTimeout(async () => {
		await startCommit(queryId, providerPort);
	}, 5 * delay * 1000);
})();

// const socket = io(`${baseUrl}/connect`);
// socket.connect();

// socket.on("connect", () => {
// 	console.log(socket.id);
// });

// socket.on("connect_error", (e) => {
// 	console.log("Oops something went wrong!", e);
// });
