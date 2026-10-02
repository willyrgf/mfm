host="${1:?missing planned host}"
http_port="${2:?missing planned HTTP port}"
peer_port="${3:?missing planned peer port}"

# The advertised address is not authoritative; connect only to the planned listener.
identity="$(curl --fail-with-body --silent --show-error --noproxy '*' \
  --header 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"admin_nodeInfo","params":[]}' \
  "http://$host:$http_port" |
  jq --exit-status --raw-output '
    if .jsonrpc != "2.0" or .id != 1 then
      error("admin_nodeInfo: invalid JSON-RPC envelope")
    elif has("error") then
      error("admin_nodeInfo: " + (.error | tojson))
    else
      .result.enode | split("@")[0]
    end')"

# Reth owns identity validation, ECIES authentication and the devp2p Hello exchange.
exec reth p2p rlpx ping "${identity}@${host}:${peer_port}" \
  --quiet --log.file.max-files 0
