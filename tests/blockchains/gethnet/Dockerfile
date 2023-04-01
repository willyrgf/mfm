FROM ethereum/client-go:stable-arm64

RUN apk --no-cache add curl

COPY password.txt /root/config/password.txt
COPY genesis.json /root/config/genesis.json
COPY data/keystore /root/.ethereum/devchain/keystore

EXPOSE 8545
EXPOSE 8546

RUN geth init /root/config/genesis.json

HEALTHCHECK --interval=10s --timeout=20s --retries=6 \
	CMD curl --fail http://localhost:8545 -X POST -H 'Content-Type: application/json' \
		--data '{"jsonrpc":"2.0", "method":"net_listening", "param":[], "id": 11077}' || exit 1


ENTRYPOINT [ "geth", "--dev", "--dev.period", "1", "--miner.gaslimit", "30000000", "--password", \
  "/root/config/password.txt", "--datadir", "/root/.ethereum/devchain", "--mine",  "--ipcdisable", \
  "--http", "--http.api", "eth,web3,personal,net,txpool", "--http.addr", "0.0.0.0", "--http.port=8545", "--http.corsdomain", "*", \
  "--ws", "--ws.origins", "*", "--ws.addr", "0.0.0.0", "--ws.port=8546", "--graphql", "--graphql.corsdomain", "*", \
  "--authrpc.vhosts", "*", "--http.vhosts", "*", \
  "--allow-insecure-unlock", "--rpc.allow-unprotected-txs", "--vmdebug", "--networkid=11077", "--rpc.txfeecap", "0", \
  "--unlock", "0x4E22e05C29A7165Aeee0D813bE03Af17F129A2d1" \
]