{ lib, pkgs, ... }:
let
  probePackage = pkgs.writeScriptBin "mfm-reth-probe" ''
    #!${pkgs.python3}/bin/python3
    ${builtins.readFile ./reth-probe.py}
  '';
  probe = args: {
    tools = [
      "reth-probe"
      "reth-managed-node"
    ];
    run = [ "mfm-reth-probe" ] ++ args;
  };
  fixture =
    name: interval:
    let
      httpProbe = probe [
        "http"
        "\${host}"
        "\${port}"
      ];
      peerProbe = probe [
        "peer"
        "\${host}"
        "\${port:${name}-http}"
        "\${port}"
      ];
      policy = {
        timeoutMs = 5000;
        retryIntervalMs = 500;
        maxAttempts = 60;
      };
    in
    {
      primaryEndpoint = "${name}-http";
      containment = "process-tree";
      endpoints = {
        "${name}-http" = {
          readyProbe = httpProbe;
          healthProbe = httpProbe;
        };
        "${name}-p2p" = {
          readyProbe = peerProbe;
          healthProbe = peerProbe;
        };
      };
      lifecycle = {
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
        ready = { inherit policy; };
        health = { inherit policy; };
        stop.timeoutMs = 10000;
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
  nixfied.closures.reth-probe = {
    package = probePackage;
    executable = "bin/mfm-reth-probe";
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
    invocation =
      (probe [
        "smoke"
        "\${host:reth}"
        "\${port:reth}"
        "\${host:reth-delayed}"
        "\${port:reth-delayed}"
      ])
      // {
        timeoutMs = 30000;
      };
  };
  nixfied.tasks.reth-probe-check.invocation = {
    tools = [
      pkgs.python3
      "reth-managed-node"
    ];
    run = [
      (baseNameOf (lib.getExe pkgs.python3))
      "nix/tests/reth_probe_test.py"
    ];
    timeoutMs = 30000;
    env.PYTHONDONTWRITEBYTECODE = "1";
  };
}
