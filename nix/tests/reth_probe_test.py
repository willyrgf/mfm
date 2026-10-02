"""Exercise protocol rejection with real HTTP/TCP servers and pinned Reth."""

import contextlib
import http.server
import importlib.util
import io
import json
import os
import pathlib
import socket
import socketserver
import subprocess
import threading
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "reth_probe", pathlib.Path(__file__).parents[1] / "reth-probe.py"
)
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)

# Public secp256k1 generator coordinates; no signing material is used by the test.
NODE_ID = (
    "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
    "483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8"
)


class RpcHandler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        self.server.methods.append(request["method"])
        self.send_response(200)
        self.end_headers()
        self.wfile.write(self.server.answer)

    def log_message(self, *args):
        pass


@contextlib.contextmanager
def rpc_server(answer):
    with http.server.ThreadingHTTPServer(("127.0.0.1", 0), RpcHandler) as server:
        server.answer = answer if isinstance(answer, bytes) else json.dumps(answer).encode()
        server.methods = []
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            yield server
        finally:
            server.shutdown()
            thread.join()


def envelope(result):
    return {"jsonrpc": "2.0", "id": 1, "result": result}


class ProbeTests(unittest.TestCase):
    def test_http_accepts_protocol_response(self):
        with rpc_server(envelope("0x2a")) as server:
            self.assertEqual(probe.block_number(*server.server_address), 42)
            self.assertEqual(server.methods, ["eth_blockNumber"])

    def test_http_ignores_ambient_proxy_routing(self):
        ambient = {
            "http_proxy": "http://192.0.2.1:1",
            "HTTP_PROXY": "http://192.0.2.1:1",
            "no_proxy": "",
            "NO_PROXY": "",
        }
        with patch.dict(os.environ, ambient), rpc_server(envelope("0x2a")) as server:
            self.assertEqual(probe.block_number(*server.server_address), 42)

    def test_http_rejects_invalid_responses(self):
        invalid = [
            b"not json", [], {"jsonrpc": "2.0", "id": 2, "result": "0x1"},
            {"jsonrpc": "2.0", "id": True, "result": "0x1"},
            {"jsonrpc": "1.0", "id": 1, "result": "0x1"},
            {"jsonrpc": "2.0", "id": 1, "error": {"code": -32601}},
            envelope(None), envelope(42), envelope("0xinvalid"),
        ]
        for answer in invalid:
            with self.subTest(answer=answer), rpc_server(answer) as server:
                with self.assertRaises(ValueError):
                    probe.block_number(*server.server_address)

    def test_rpc_error_retains_operation_and_supplied_diagnostics(self):
        error = {
            "code": -32000,
            "message": "native failure",
            "data": {"cause": "provider rejected"},
        }
        with rpc_server({"jsonrpc": "2.0", "id": 1, "error": error}) as server:
            with self.assertRaises(ValueError) as failure:
                probe.block_number(*server.server_address)
        for detail in ["eth_blockNumber", "-32000", "native failure", "provider rejected"]:
            self.assertIn(detail, str(failure.exception))

    def test_peer_rejects_invalid_node_identity(self):
        invalid = [
            None, {}, {"enode": "bad"}, {"enode": 123},
            {"enode": f"enode://{'g' * 128}@127.0.0.1:1"},
            {"enode": f"enode://{NODE_ID[:-2]}@127.0.0.1:1"},
            {"enode": f"enode://{NODE_ID}"},
        ]
        for result in invalid:
            with self.subTest(result=result), rpc_server(envelope(result)) as server:
                with self.assertRaisesRegex(ValueError, "public node identity"):
                    probe.peer(*server.server_address, 1)

    def test_peer_rejects_closed_port(self):
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            closed_port = reservation.getsockname()[1]
        with rpc_server(envelope({"enode": f"enode://{NODE_ID}@127.0.0.1:1"})) as server:
            with self.assertRaises(subprocess.CalledProcessError):
                probe.peer(*server.server_address, closed_port)

    def test_peer_failure_retains_original_cause_and_stderr(self):
        command = ["reth", "p2p", "rlpx", "ping"]
        node_info = {"enode": f"enode://{NODE_ID}@127.0.0.1:1"}
        originals = [
            subprocess.TimeoutExpired(command, 2, stderr=b"native handshake context\n"),
            subprocess.CalledProcessError(1, command, stderr="native handshake rejection\n"),
        ]
        for original in originals:
            with (
                self.subTest(cause=type(original)),
                io.BytesIO() as captured,
                io.TextIOWrapper(captured) as output,
            ):
                with (
                    patch.object(probe, "rpc", return_value=node_info),
                    patch.object(probe.subprocess, "run", side_effect=original),
                    patch.object(probe.sys, "stderr", output),
                    self.assertRaises(type(original)) as failure,
                ):
                    probe.peer("127.0.0.1", 1, 2)
                self.assertIs(failure.exception, original)
                output.flush()
                expected = (
                    original.stderr.encode()
                    if isinstance(original.stderr, str)
                    else original.stderr
                )
                self.assertEqual(captured.getvalue(), expected)

    def test_peer_rejects_plain_tcp_and_ignores_advertised_address(self):
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
                node_info = {
                    "id": "compressed-node-key",
                    "enode": f"enode://{NODE_ID}@192.0.2.1:1",
                }
                with rpc_server(envelope(node_info)) as server:
                    with self.assertRaises(subprocess.CalledProcessError):
                        probe.peer(*server.server_address, peer_server.server_address[1])
                self.assertEqual(peer_server.connections, 1)
            finally:
                peer_server.shutdown()
                thread.join()


if __name__ == "__main__":
    unittest.main()
