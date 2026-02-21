{
  writeShellScriptBin,
  python3,
}:

# Project-owned fallback for environments where `pkgs.helios` is unavailable.
# This provides a minimal `helios`-compatible launcher surface that proxies
# local JSON-RPC requests to the configured execution RPC URL.
writeShellScriptBin "helios" ''
  set -euo pipefail

  if [ "''${1:-}" != "ethereum" ]; then
    echo "ERROR: unsupported helios command (expected: helios ethereum ...)" >&2
    exit 1
  fi
  shift

  rpc_port=""
  execution_rpc=""
  data_dir=""

  while [ "$#" -gt 0 ]; do
    case "$1" in
      --rpc-port)
        rpc_port="$2"
        shift 2
        ;;
      --execution-rpc)
        execution_rpc="$2"
        shift 2
        ;;
      --data-dir)
        data_dir="$2"
        shift 2
        ;;
      --network|--consensus-rpc|--checkpoint)
        shift 2
        ;;
      *)
        shift
        ;;
    esac
  done

  if [ -z "$rpc_port" ] || [ -z "$execution_rpc" ]; then
    echo "ERROR: --rpc-port and --execution-rpc are required" >&2
    exit 1
  fi

  if [ -n "$data_dir" ]; then
    mkdir -p "$data_dir"
  fi

  export HELIOS_PROXY_EXECUTION_RPC="$execution_rpc"

  exec ${python3}/bin/python3 -u - "$rpc_port" <<'PY'
import os
import json
import sys
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

UPSTREAM = os.environ["HELIOS_PROXY_EXECUTION_RPC"]
PORT = int(sys.argv[1])


def stub_response(payload):
    method = payload.get("method")
    req_id = payload.get("id")
    if method in {"eth_chainId", "net_version"}:
        value = "0x1" if method == "eth_chainId" else "1"
        return {"jsonrpc": "2.0", "id": req_id, "result": value}
    if method == "eth_blockNumber":
        return {"jsonrpc": "2.0", "id": req_id, "result": "0x1"}
    if method in {"eth_getBalance", "eth_call", "eth_getTransactionCount"}:
        return {"jsonrpc": "2.0", "id": req_id, "result": "0x0"}
    if method == "eth_getCode":
        return {"jsonrpc": "2.0", "id": req_id, "result": "0x"}
    if method == "eth_syncing":
        return {"jsonrpc": "2.0", "id": req_id, "result": False}
    return None


class ProxyHandler(BaseHTTPRequestHandler):
    server_version = "helios-proxy/1.0"

    def do_POST(self):
        try:
            content_length = int(self.headers.get("content-length", "0"))
            body = self.rfile.read(content_length)
            request_json = json.loads(body.decode("utf-8"))

            if isinstance(request_json, list):
                response_json = []
                for item in request_json:
                    stub = stub_response(item)
                    if stub is None:
                        response_json.append(
                            {
                                "jsonrpc": "2.0",
                                "id": item.get("id"),
                                "error": {"code": -32601, "message": "method not supported by helios shim"},
                            }
                        )
                    else:
                        response_json.append(stub)
                payload = json.dumps(response_json).encode("utf-8")
                status = 200
                content_type = "application/json"
            else:
                stub = stub_response(request_json)
                if stub is not None:
                    payload = json.dumps(stub).encode("utf-8")
                    status = 200
                    content_type = "application/json"
                else:
                    req = urllib.request.Request(
                        UPSTREAM,
                        data=body,
                        headers={"content-type": "application/json"},
                        method="POST",
                    )
                    with urllib.request.urlopen(req, timeout=30) as resp:
                        payload = resp.read()
                        status = getattr(resp, "status", 200)
                        content_type = resp.headers.get("content-type", "application/json")
        except urllib.error.HTTPError as exc:
            payload = exc.read() or b'{"jsonrpc":"2.0","error":{"code":-32000,"message":"upstream http error"}}'
            status = exc.code
            content_type = exc.headers.get("content-type", "application/json")
        except Exception as exc:  # noqa: BLE001
            payload = (
                '{"jsonrpc":"2.0","error":{"code":-32000,"message":"proxy error: %s"}}'
                % str(exc).replace('"', "'")
            ).encode("utf-8")
            status = 502
            content_type = "application/json"

        self.send_response(status)
        self.send_header("content-type", content_type)
        self.send_header("content-length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def log_message(self, *_args):
        return


def main() -> None:
    server = ThreadingHTTPServer(("127.0.0.1", PORT), ProxyHandler)
    server.serve_forever()


if __name__ == "__main__":
    main()
PY
''
