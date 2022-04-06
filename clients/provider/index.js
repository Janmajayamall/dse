const { io } = require("socket.io-client");
const axios = require("axios");
const fetch = require("node-fetch");

const baseUrl = `http://127.0.0.1:${process.env.PORT}`;
const baseInstance = axios.create({
	baseUrl: baseUrl,
	timeout: 10000,
	headers: { "Content-Type": "application/json" },
	Proxy: undefined,
});

async function sendQuery(query) {
	const res = await fetch(`${baseUrl}/action`, {
		method: "post",
		body: JSON.stringify({ NewQuery: { query: { query: query } } }),
		headers: { "Content-Type": "application/json" },
	});
	console.log(await res);
}

async function sendBid(queryId, requesterId) {
	const res = await fetch(`${baseUrl}/action`, {
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

async function acceptBid(queryId, bidderId) {
	const res = await fetch(`${baseUrl}/action`, {
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

async function startCommit(queryId) {
	const res = await fetch(`${baseUrl}/action`, {
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

async function receivedQueries() {
	const res = await fetch(`${baseUrl}/recvqueries`, {
		method: "get",
	});
	console.log(await res.json());
}

async function getQueryBids(queryId) {
	const res = await fetch(`${baseUrl}/querybids/${queryId}`, {
		method: "get",
	});
	console.log(await res.json());
}

(async () => {
	// await sendQuery("yolo2");
	// await receivedQueries();
	// await sendBid(1, "16Uiu2HAm4ro25Yb85MzJcDdfTyks4NEgLSBTTnHs4UPcAmPg65ah");
	// await getQueryBids(1);
	// await acceptBid(1, "16Uiu2HAmMjYbNBrHYx9K7MgQnfRaUgKPZeRV1zvPxttCFxGSzasN");
	await startCommit(1);
})();

// const socket = io(`${baseUrl}/connect`);
// socket.connect();

// socket.on("connect", () => {
// 	console.log(socket.id);
// });

// socket.on("connect_error", (e) => {
// 	console.log("Oops something went wrong!", e);
// });
