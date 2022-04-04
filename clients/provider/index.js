const { io } = require("socket.io-client");
const axios = require("axios");

const PORT = 3000;
const baseUrl = `http://127.0.0.1:${PORT}`;
console.log(baseUrl, " this is the ba");
const baseInstance = axios.create({
	baseUrl: "http://127.0.0.1:3000",
	timeout: 10000,
	headers: { "Content-Type": "application/json" },
});

async function sendQuery() {
	const { data } = await baseInstance.request({
		url: "/newquery",
		method: "POST",
		data: {
			query: "jdiwajdioa",
		},
	});
}
(async () => {
	await sendQuery();
})();

// const socket = io(`${baseUrl}/connect`);
// socket.connect();

// socket.on("connect", () => {
// 	console.log(socket.id);
// });

// socket.on("connect_error", (e) => {
// 	console.log("Oops something went wrong!", e);
// });
