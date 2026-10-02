"""Check the local mapping from RPC identity to a planned RLPx listener."""

import contextlib
import http.server
import json
import socket
import socketserver
import subprocess
import sys
import threading
import unittest

# Public secp256k1 generator coordinates; no signing material is used by the test.
NODE_ID = (
    "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
    "483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8"
)
PEER_PROBE = sys.argv[1]
NODE_RESPONSE = {
    "jsonrpc": "2.0", "id": 1,
    "result": {
        "id": "compressed-node-key",
        "enode": f"enode://{NODE_ID}@192.0.2.1:1",
    },
}


class RpcHandler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers["Content-Length"]))
        self.send_response(200)
        self.end_headers()
        self.wfile.write(self.server.answer)

    def log_message(self, *args):
        pass


@contextlib.contextmanager
def rpc_server(answer=NODE_RESPONSE):
    with http.server.ThreadingHTTPServer(("127.0.0.1", 0), RpcHandler) as server:
        server.answer = json.dumps(answer).encode()
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            yield server.server_address
        finally:
            server.shutdown()
            thread.join()


class PeerTests(unittest.TestCase):
    def test_identity_rpc_error_preserves_operation_code_message_and_data(self):
        answer = {
            "jsonrpc": "2.0", "id": 1,
            "error": {
                "code": -32000, "message": "native failure",
                "data": {"cause": "identity unavailable"},
            },
        }
        with rpc_server(answer) as (host, http_port):
            result = subprocess.run(
                [PEER_PROBE, host, str(http_port), "1"],
                capture_output=True, text=True, timeout=8,
            )
        self.assertNotEqual(result.returncode, 0)
        for detail in ["admin_nodeInfo", "-32000", "native failure", "identity unavailable"]:
            self.assertIn(detail, result.stderr)

    def test_closed_planned_port_preserves_native_connection_failure(self):
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            closed_port = reservation.getsockname()[1]
        with rpc_server() as (host, http_port):
            result = subprocess.run(
                [PEER_PROBE, host, str(http_port), str(closed_port)],
                capture_output=True, text=True, timeout=8,
            )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Connection refused", result.stderr)

    def test_plain_tcp_fails_handshake_at_planned_not_advertised_address(self):
        class PlainTcp(socketserver.BaseRequestHandler):
            def handle(self):
                self.server.connections += 1
                self.request.recv(4096)
                self.request.sendall(b"not an RLPx handshake")

        with socketserver.TCPServer(("127.0.0.1", 0), PlainTcp) as peer_server:
            peer_server.connections = 0
            thread = threading.Thread(target=peer_server.serve_forever, daemon=True)
            thread.start()
            try:
                with rpc_server() as (host, http_port):
                    result = subprocess.run(
                        [PEER_PROBE, host, str(http_port),
                         str(peer_server.server_address[1])],
                        capture_output=True, text=True, timeout=8,
                    )
                self.assertNotEqual(result.returncode, 0)
                self.assertTrue(result.stderr)
                self.assertEqual(peer_server.connections, 1)
            finally:
                peer_server.shutdown()
                thread.join()


if __name__ == "__main__":
    unittest.main(argv=[sys.argv[0]])
