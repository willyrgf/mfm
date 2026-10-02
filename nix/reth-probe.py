"""Readiness for the two loopback Reth fixture listeners."""

import json
import re
import subprocess
import sys
import time
import traceback
import urllib.request


def rpc(host, port, method):
    request = urllib.request.Request(
        f"http://{host}:{port}",
        data=json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": []}).encode(),
        headers={"Content-Type": "application/json"},
    )
    # Fixture readiness must not inherit caller proxy routing.
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(request, timeout=1) as response:
        payload = json.load(response)
    if (
        not isinstance(payload, dict)
        or payload.get("jsonrpc") != "2.0"
        or type(payload.get("id")) is not int
        or payload["id"] != 1
    ):
        raise ValueError(f"{method}: invalid JSON-RPC response envelope")
    if "error" in payload:
        raise ValueError(f"{method}: JSON-RPC error: {json.dumps(payload['error'])}")
    if "result" not in payload:
        raise ValueError(f"{method}: JSON-RPC response has no result")
    return payload["result"]


def block_number(host, port):
    result = rpc(host, port, "eth_blockNumber")
    if not isinstance(result, str) or not re.fullmatch(r"0x[0-9a-fA-F]+", result):
        raise ValueError("eth_blockNumber: invalid block quantity")
    return int(result, 16)


def peer(host, http_port, peer_port):
    result = rpc(host, http_port, "admin_nodeInfo")
    advertised = result.get("enode") if isinstance(result, dict) else None
    identity = (
        re.match(r"enode://([0-9a-fA-F]{128})@", advertised)
        if isinstance(advertised, str)
        else None
    )
    if identity is None:
        raise ValueError("admin_nodeInfo: invalid public node identity")
    # Advertised addresses cannot redirect a probe away from its planned endpoint.
    enode = f"enode://{identity[1]}@{host}:{peer_port}"
    try:
        subprocess.run(
            ["reth", "p2p", "rlpx", "ping", enode, "--quiet", "--log.file.max-files", "0"],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE, text=True, timeout=2, check=True,
        )
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
        if error.stderr:
            if isinstance(error.stderr, bytes):
                sys.stderr.buffer.write(error.stderr)
            else:
                sys.stderr.write(error.stderr)
        raise


def main(args):
    mode, *addresses = args
    if mode == "http" and len(addresses) == 2:
        block_number(*addresses)
    elif mode == "peer" and len(addresses) == 3:
        peer(*addresses)
    elif mode == "smoke" and len(addresses) == 4:
        host, http_port, delayed_host, delayed_http = addresses
        block_number(host, http_port)
        # Service admission already qualified both peer listeners and protocols.
        before = block_number(delayed_host, delayed_http)
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            time.sleep(0.5)
            if block_number(delayed_host, delayed_http) > before:
                return
        raise RuntimeError("delayed fixture did not advance its block number")
    else:
        raise ValueError(
            "expected http HOST PORT, peer HOST HTTP_PORT PEER_PORT, "
            "or smoke HOST HTTP_PORT DELAYED_HOST DELAYED_HTTP_PORT"
        )


if __name__ == "__main__":
    try:
        main(sys.argv[1:])
    except Exception:
        traceback.print_exc()
        sys.exit(1)
