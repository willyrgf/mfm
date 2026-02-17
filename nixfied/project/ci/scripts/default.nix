{ pkgs }:
{
  setup = import ./setup.nix { inherit pkgs; };
  teardown = import ./teardown.nix { inherit pkgs; };
  steps = {
    "architecture-verify" = import ./steps/architecture-verify.nix { inherit pkgs; };
    "audit" = import ./steps/audit.nix { inherit pkgs; };
    "build" = import ./steps/build.nix { inherit pkgs; };
    "clippy" = import ./steps/clippy.nix { inherit pkgs; };
    "fmt" = import ./steps/fmt.nix { inherit pkgs; };
    "mainnet-portfolio-snapshot-helios" = import ./steps/mainnet-portfolio-snapshot-helios.nix { inherit pkgs; };
    "parity-aave-v3-reth" = import ./steps/parity-aave-v3-reth.nix { inherit pkgs; };
    "parity-evm-helios-smoke" = import ./steps/parity-evm-helios-smoke.nix { inherit pkgs; };
    "parity-evm-reth" = import ./steps/parity-evm-reth.nix { inherit pkgs; };
    "parity-keystore-reth-tx-sign-send" = import ./steps/parity-keystore-reth-tx-sign-send.nix { inherit pkgs; };
    "parity-portfolio-tracker-reth" = import ./steps/parity-portfolio-tracker-reth.nix { inherit pkgs; };
    "parity-postgres" = import ./steps/parity-postgres.nix { inherit pkgs; };
    "parity-rest-api-smoke" = import ./steps/parity-rest-api-smoke.nix { inherit pkgs; };
    "parity-s3" = import ./steps/parity-s3.nix { inherit pkgs; };
    "shell-app-contracts" = import ./steps/shell-app-contracts.nix { inherit pkgs; };
    "tests" = import ./steps/tests.nix { inherit pkgs; };
  };
}
