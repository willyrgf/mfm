{
  lib,
  pkgs,
  adapters,
  ...
}:
let
  upstream = (adapters.reth { inherit pkgs; }).nixfied;
  peerPackage = pkgs.writeShellApplication {
    name = "mfm-reth-peer-probe";
    runtimeInputs = [
      pkgs.curl
      pkgs.jq
      pkgs.reth
    ];
    text = builtins.readFile ./reth-peer-probe.sh;
  };
  fixture =
    name: interval:
    let
      peerProbe = {
        tools = [ "reth-peer-probe" ];
        run = [
          "mfm-reth-peer-probe"
          "\${host}"
          "\${port:${name}-http}"
          "\${port}"
        ];
      };
    in
    {
      primaryEndpoint = "${name}-http";
      containment = "process-tree";
      endpoints = {
        "${name}-http" = upstream.services.reth.endpoints.reth-http;
        "${name}-p2p" = {
          readyProbe = peerProbe;
          healthProbe = peerProbe;
        };
      };
      lifecycle = upstream.services.reth.lifecycle // {
        start.invocation = {
          tools = [ "reth-managed-node" ];
          run = [
            "reth"
            "node"
            "--dev"
            "--datadir"
            "\${stateDir}/${name}/data"
            "--ipcdisable"
            "--disable-discovery"
            "--disable-auth-server"
            "--addr"
            "127.0.0.1"
            "--port"
            "\${port:${name}-p2p}"
            # The authenticated readiness handshake needs one inbound peer.
            "--max-inbound-peers"
            "1"
            "--max-outbound-peers"
            "0"
            "--http"
            "--http.addr"
            "127.0.0.1"
            "--http.port"
            "\${port:${name}-http}"
            "--http.api"
            "eth,admin"
            "--quiet"
            "--log.file.max-files"
            "0"
          ]
          ++ lib.optionals interval [
            "--dev.block-time"
            "10s"
          ];
        };
      };
    };
in
{
  nixfied.closures.reth-managed-node = {
    package = pkgs.reth;
    executable = "bin/reth";
    effects = [
      "process"
      "network-listener"
      "file-write"
    ];
  };
  nixfied.closures.reth-rpc-probe = upstream.closures.reth-rpc-probe;
  nixfied.closures.reth-peer-probe = {
    package = peerPackage;
    executable = "bin/mfm-reth-peer-probe";
    effects = [ "process" ];
  };
  nixfied.services = {
    reth = fixture "reth" false;
    reth-delayed = fixture "reth-delayed" true;
  };
  nixfied.tasks.reth-smoke = {
    requires = [
      "reth"
      "reth-delayed"
    ];
    invocation = upstream.tasks.reth-smoke.invocation // {
      run = [
        "nixfied-reth-probe"
        "http"
        "\${host:reth}"
        "\${port:reth}"
      ];
    };
  };
}
