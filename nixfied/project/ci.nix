{
  pkgs,
  project,
  lib,
  ...
}:

let
  # v2 shell-app contract inventory (project-level):
  # - ci: batch-runner, outputs=text, wraps CI shell runner + cargo tools, failure map owner=project/ci.nix
  failureCodesScript = {
    generic = 1;
    usage = 2;
    precondition = 3;
    unavailable = 4;
    timeout = 5;
  };
  failureCodesCargo = failureCodesScript // {
    cargoTestFailure = 100;
    cargoFailure = 101;
  };
  ciModeValues = [
    "basic"
    "audit"
    "parity"
    "full"
    "mainnet"
  ];
  ciAppContract = {
    version = 2;
    name = "ci";
    commandClass = "batch-runner";
    allowUnknownArgs = false;
    idempotent = false;
    failureCodes = failureCodesCargo;
    outputs = {
      mode = "text";
    };
    args = [
      {
        name = "basic";
        kind = "flag";
        long = "--basic";
        type = "bool";
        required = false;
      }
      {
        name = "audit";
        kind = "flag";
        long = "--audit";
        type = "bool";
        required = false;
      }
      {
        name = "parity";
        kind = "flag";
        long = "--parity";
        type = "bool";
        required = false;
      }
      {
        name = "full";
        kind = "flag";
        long = "--full";
        type = "bool";
        required = false;
      }
      {
        name = "mainnet";
        kind = "flag";
        long = "--mainnet";
        type = "bool";
        required = false;
      }
      {
        name = "mode";
        kind = "option";
        long = "--mode";
        type = "enum";
        values = ciModeValues;
        required = false;
      }
      {
        name = "summary";
        kind = "flag";
        long = "--summary";
        type = "bool";
        required = false;
      }
      {
        name = "background";
        kind = "flag";
        long = "--background";
        type = "bool";
        required = false;
      }
      {
        name = "bg";
        kind = "flag";
        long = "--bg";
        type = "bool";
        required = false;
      }
    ];
    env = [
      {
        name = "CI_ARTIFACTS_DIR";
        type = "string";
        required = false;
      }
      {
        name = "CI_ARTIFACTS_BASE";
        type = "string";
        required = false;
      }
      {
        name = "SERVICE_REUSE_POLICY";
        type = "enum";
        values = [
          "never"
          "same-root"
          "same-slot"
          "cross-run"
        ];
        required = false;
      }
      {
        name = "SERVICE_OWNER_SCOPE";
        type = "enum";
        values = [
          "ephemeral"
          "persistent"
        ];
        required = false;
      }
      {
        name = "SERVICE_DISCOVERY_SCOPE";
        type = "enum";
        values = [
          "local"
          "global"
        ];
        required = false;
      }
    ];
  };
  # Policy: CI shell logic must be Nix-packaged/Nix-evaluated (no committed raw .sh scripts).
  ciScripts = import ./ci/scripts { inherit pkgs; };
  ciSetupScript = toString ciScripts.setup;
  ciTeardownScript = toString ciScripts.teardown;
  ciStepScript = name: toString ciScripts.steps.${name};
in
{
  commands = {
    ci = {
      description = "Run the CI pipeline";
      api = lib.appApi.mkApi {
        name = "ci";
        summary = "Run the CI pipeline";
        details = ''
          Runs the CI pipeline defined in `nixfied/project/ci.nix` (modes + steps).

          Supported options (from the framework CI runner):
          - `--basic|--audit|--parity|--full|--mainnet`: select a mode
          - `--mode <name>` or `--mode=<name>`: select a mode by name
          - `--summary`: print a compact summary and write `summary.json` into the artifacts dir
          - `--bg|--background`: run in background via the run registry
          - `LOG_LEVEL`: canonical process-wide baseline log level/filter for Rust apps/tests/services launched by CI
            (runtime env, not a CI app-contract field)
          - `OUTPUT_MODE`: canonical process log routing (`stdout|logs|both`) for CI-launched apps/services
            (runtime env, not a CI app-contract field)

          Readiness-first behavior:
          - parity/mainnet workflows gate on service `*_READY` hooks
          - parity fixtures with `profile = "test"` use profile-specific readiness when available (Postgres uses `SVC_POSTGRES_READY_TEST`)
          - fixture logs are persisted under CI artifacts for debugging

          Service diagnostics coverage:
          - CI setup normalizes logging env for Rust apps/tests and service wrappers.
          - Canonical CI env vars are `LOG_LEVEL` and `OUTPUT_MODE`; compatibility aliases `NIXFIED_LOG_LEVEL` and `NIXFIED_OUTPUT_MODE` are supported.
          - Legacy CI env vars are rejected (`CI_LOG_LEVEL`, `CI_VERBOSE`, `NIXFIED_VERBOSE`, `NIXFIED_DEBUG`).
          - If output mode is unset and `LOG_LEVEL=debug`, CI defaults `OUTPUT_MODE=both`.
          - CI teardown collects diagnostics for `postgres`, `minio`, `reth`, `helios`, and `nginx`.
          - service status/events are always captured; service log tails are captured when `LOG_LEVEL` resolves to `debug` or `trace` (independent of `OUTPUT_MODE`).

          Process-first diagnostics:
          - `nix run .#process::status -- --all`
          - `nix run .#process::runs -- --all`
          - `nix run .#process::inspect -- <run-id>`
          - `nix run .#svc::postgres::events -- --limit 100`
        '';
        usage = [
          "nix run .#ci -- --basic --summary"
          "nix run .#ci -- --audit --summary"
          "nix run .#ci -- --parity --summary"
          "nix run .#ci -- --full --summary"
          "nix run .#ci -- --mode basic --summary"
          "nix run .#ci -- --mode full --summary"
          "nix run .#ci -- --mode=basic --summary"
          "LOG_LEVEL='debug,mfm=trace' nix run .#ci -- --parity --summary"
          "OUTPUT_MODE='both' LOG_LEVEL='debug,mfm=trace' nix run .#ci -- --parity --summary"
          "nix run .#ci -- --bg"
          "nix run .#ci -- --background"
        ];
        examples = [
          "nix run .#ci -- --basic --summary"
          "CI_ARTIFACTS_DIR=/tmp/ci-artifacts nix run .#ci -- --parity --summary"
          "LOG_LEVEL='debug,mfm=trace' nix run .#ci -- --parity --summary"
          "OUTPUT_MODE='logs' LOG_LEVEL='debug,mfm=trace' nix run .#ci -- --parity --summary"
          "nix run .#ci -- --full --summary"
          "nix run .#process::status -- --all"
        ];
        args = [
          {
            name = "--basic";
            description = "Select basic mode.";
          }
          {
            name = "--audit";
            description = "Select audit mode.";
          }
          {
            name = "--parity";
            description = "Select parity mode.";
          }
          {
            name = "--full";
            description = "Select full mode (basic + parity).";
          }
          {
            name = "--mainnet";
            description = "Select mainnet mode.";
          }
          {
            name = "--mode";
            description = "Select CI mode by name (value: <name>).";
          }
          {
            name = "--summary";
            description = "Print a compact summary and write artifacts/summary.json.";
          }
          {
            name = "--bg";
            description = "Run CI in background via the run registry.";
          }
          {
            name = "--background";
            description = "Alias for --bg.";
          }
        ];
        env = [
          {
            name = "CI_ARTIFACTS_DIR";
            description = "Override artifacts directory (default: /tmp/ci-artifacts).";
          }
          {
            name = "CI_ARTIFACTS_BASE";
            description = "Override artifacts root directory (must be absolute path).";
          }
          {
            name = "SERVICE_REUSE_POLICY";
            description = "Optional process-first policy override (never|same-root|same-slot|cross-run).";
          }
          {
            name = "SERVICE_OWNER_SCOPE";
            description = "Optional process-first policy override (ephemeral|persistent).";
          }
          {
            name = "SERVICE_DISCOVERY_SCOPE";
            description = "Optional process-first policy override (local|global).";
          }
        ];
        category = "core";
        allowUnknownArgs = false;
        failureCodes = failureCodesCargo;
        idempotent = false;
        appContract = ciAppContract;
      };
      env = {
        "${project.envVar}" = "test";
      };
      useDeps = true;
      # Note: `nix run .#ci` is implemented by the framework CI runner (nixfied/.framework/ci.nix),
      # which reads `project.ci.*` below. This command exists for `nix run .#help`, and
      # `commands.ci.api` is the canonical CI docs source mirrored into app metadata.
      script = "";
    };
  };

  ci = {
    enable = true;
    defaultMode = "basic";
    env = {
      "${project.envVar}" = "test";
      CARGO_TERM_COLOR = "always";
      # Keep rust artifacts in the run-scoped ephemeral root so sequential CI
      # steps can reuse them safely without cross-run leakage.
      CARGO_TARGET_DIR = "$MFM_EPHEMERAL_ROOT/build/cargo-target";
      CARGO_HOME = "$MFM_EPHEMERAL_ROOT/build/cargo-home";
      RUSTC_WRAPPER = "sccache";
      SCCACHE_DIR = "$HOME/.cache/mfm/sccache";
      RUST_BACKTRACE = "1";
      SERVICE_OWNER_SCOPE = "persistent";
      SERVICE_DISCOVERY_SCOPE = "global";
      SERVICE_REUSE_POLICY = "same-slot";
      # MinIO accepts both ROOTDISK/ROOTDRIVE variable names across releases.
      # These vars control root-drive classification, not the object-write fill fraction.
      MINIO_ROOTDISK_THRESHOLD_SIZE = "128MiB";
      MINIO_ROOTDRIVE_THRESHOLD_SIZE = "128MiB";
    };
    useDeps = true;
    setupActions = [
      {
        kind = "exec";
        argv = [
          "."
          ciSetupScript
        ];
      }
    ];
    teardownActions = [
      {
        kind = "exec";
        argv = [
          "."
          ciTeardownScript
        ];
      }
    ];
    failureSignals = [ ];
    runsRoot = "/tmp/${project.id}-runs";
    useEphemeral = true;
    artifacts = {
      dir = "/tmp/ci-artifacts";
      keepOnFailure = true;
      keepOnSuccess = false;
    };
    modes = {
      basic = {
        steps = [
          "fmt"
          "clippy"
          "architecture-verify"
          "shell-app-contracts"
          "build"
          "tests"
        ];
      };
      audit = {
        steps = [ "audit" ];
      };
      parity = {
        steps = [
          "parity-postgres"
          "parity-s3"
          "parity-rest-api-smoke"
          "parity-evm-reth"
          "parity-aave-v3-reth"
          "parity-postgres-state-events-audit"
          "parity-keystore-reth-tx-sign-send"
          "parity-evm-helios-smoke"
        ];
      };
      full = {
        steps = [
          "fmt"
          "clippy"
          "architecture-verify"
          "shell-app-contracts"
          "build"
          "tests"
          "parity-postgres"
          "parity-s3"
          "parity-rest-api-smoke"
          "parity-evm-reth"
          "parity-aave-v3-reth"
          "parity-keystore-reth-tx-sign-send"
          "parity-evm-helios-smoke"
        ];
      };
      mainnet = {
        steps = [ "mainnet-portfolio-snapshot-helios" ];
      };
    };
    steps = {
      tests = {
        description = "Tests";
        actions = [
          {
            kind = "exec";
            argv = [
              "."
              (ciStepScript "tests")
            ];
          }
        ];
      };
      fmt = {
        description = "Formatting (nightly)";
        actions = [
          {
            kind = "exec";
            argv = [
              "."
              (ciStepScript "fmt")
            ];
          }
        ];
      };
      clippy = {
        description = "Clippy (nightly)";
        actions = [
          {
            kind = "exec";
            argv = [
              "."
              (ciStepScript "clippy")
            ];
          }
        ];
      };
      architecture-verify = {
        description = "Architecture verifier";
        actions = [
          {
            kind = "exec";
            argv = [
              "."
              (ciStepScript "architecture-verify")
            ];
          }
        ];
      };
      shell-app-contracts = {
        description = "Shell app contract checks (typed/json/batch-runner/passthrough)";
        actions = [
          {
            kind = "exec";
            argv = [
              "."
              (ciStepScript "shell-app-contracts")
            ];
          }
        ];
      };
      build = {
        description = "Build (release, all features)";
        actions = [
          {
            kind = "exec";
            argv = [
              "."
              (ciStepScript "build")
            ];
          }
        ];
      };
      audit = {
        description = "Security audit (cargo-audit)";
        actions = [
          {
            kind = "exec";
            argv = [
              "."
              (ciStepScript "audit")
            ];
          }
        ];
      };

      parity-postgres = {
        description = "Parity: Postgres event store";
        env = {
          AUTO_STOP_CONFLICTING = "1";
        };
        fixtures = {
          services = [
            {
              name = "postgres";
              profile = "test";
            }
          ];
          artifacts = {
            logs = true;
            prefix = "parity-postgres";
          };
        };
        actions = [
          {
            kind = "exec";
            argv = [
              "."
              (ciStepScript "parity-postgres")
            ];
          }
        ];
      };

      parity-s3 = {
        description = "Parity: S3/MinIO artifact store";
        fixtures = {
          services = [
            {
              name = "minio";
              profile = "test";
              exports = [ "s3" ];
              bucket = "mfm-test";
              prefix = "mfm-artifacts";
              region = "us-east-1";
              bootstrap = [ "mfm-test" ];
              logName = "minio.log";
            }
          ];
          artifacts = {
            logs = true;
            prefix = "parity-s3";
          };
        };
        actions = [
          {
            kind = "exec";
            argv = [
              "."
              (ciStepScript "parity-s3")
            ];
          }
        ];
      };

      parity-rest-api-smoke = {
        description = "Parity: REST API smoke on Postgres + S3/MinIO";
        env = {
          AUTO_STOP_CONFLICTING = "1";
        };
        fixtures = {
          services = [
            {
              name = "postgres";
              profile = "test";
            }
            {
              name = "minio";
              profile = "test";
              exports = [ "s3" ];
              bucket = "mfm-test";
              prefix = "mfm-artifacts";
              region = "us-east-1";
              bootstrap = [ "mfm-test" ];
              logName = "minio-rest-api-smoke.log";
            }
          ];
          artifacts = {
            logs = true;
            prefix = "parity-rest-api-smoke";
          };
        };
        actions = [
          {
            kind = "exec";
            argv = [
              "."
              (ciStepScript "parity-rest-api-smoke")
            ];
          }
        ];
      };

      parity-evm-reth = {
        description = "Parity: EVM + portfolio tracker scenarios on reth";
        env = {
          AUTO_STOP_CONFLICTING = "1";
        };
        fixtures = {
          services = [
            {
              name = "postgres";
              profile = "test";
            }
            {
              name = "minio";
              profile = "test";
              exports = [ "s3" ];
              bucket = "mfm-test";
              prefix = "mfm-artifacts";
              region = "us-east-1";
              bootstrap = [ "mfm-test" ];
              logName = "minio-evm.log";
            }
            {
              name = "reth";
              profile = "test";
              logName = "reth.log";
            }
          ];
          artifacts = {
            logs = true;
            prefix = "parity-evm-reth";
          };
        };
        actions = [
          {
            kind = "exec";
            argv = [
              "."
              (ciStepScript "parity-evm-reth")
            ];
          }
        ];
      };

      parity-aave-v3-reth = {
        description = "Parity: Aave v3 deploy/configure/lend/borrow scenario on reth";
        env = {
          AUTO_STOP_CONFLICTING = "1";
        };
        fixtures = {
          services = [
            {
              name = "postgres";
              profile = "test";
            }
            {
              name = "minio";
              profile = "test";
              exports = [ "s3" ];
              bucket = "mfm-test";
              prefix = "mfm-artifacts";
              region = "us-east-1";
              bootstrap = [ "mfm-test" ];
              logName = "minio-aave-v3.log";
            }
            {
              name = "reth";
              profile = "test";
              logName = "reth-aave-v3.log";
            }
          ];
          artifacts = {
            logs = true;
            prefix = "parity-aave-v3-reth";
          };
        };
        actions = [
          {
            kind = "exec";
            argv = [
              "."
              (ciStepScript "parity-aave-v3-reth")
            ];
          }
        ];
      };

      parity-postgres-state-events-audit = {
        description = "Parity: query-only audit of Postgres kernel state events from prior parity runs";
        env = {
          AUTO_STOP_CONFLICTING = "1";
        };
        fixtures = {
          services = [
            {
              name = "postgres";
              profile = "test";
            }
          ];
          artifacts = {
            logs = true;
            prefix = "parity-postgres-state-events-audit";
          };
        };
        actions = [
          {
            kind = "exec";
            argv = [
              "."
              (ciStepScript "parity-postgres-state-events-audit")
            ];
          }
        ];
      };

      parity-keystore-reth-tx-sign-send = {
        description = "Parity: keystore CLI import/sign/send raw tx on reth";
        env = {
          AUTO_STOP_CONFLICTING = "1";
        };
        fixtures = {
          services = [
            {
              name = "reth";
              profile = "test";
              logName = "reth-keystore-cli.log";
            }
          ];
          artifacts = {
            logs = true;
            prefix = "parity-keystore-reth-tx-sign-send";
          };
        };
        actions = [
          {
            kind = "exec";
            argv = [
              "."
              (ciStepScript "parity-keystore-reth-tx-sign-send")
            ];
          }
        ];
      };

      parity-evm-helios-smoke = {
        description = "Parity: Helios lifecycle and RPC smoke over reth execution";
        fixtures = {
          services = [
            {
              name = "reth";
              profile = "test";
              logName = "reth-helios.log";
            }
          ];
          artifacts = {
            logs = true;
            prefix = "parity-evm-helios-smoke";
          };
        };
        actions = [
          {
            kind = "exec";
            argv = [
              "."
              (ciStepScript "parity-evm-helios-smoke")
            ];
          }
        ];
      };

      mainnet-portfolio-snapshot-helios = {
        description = "Mainnet: mfm::portfolio::snapshot (Helios) and assert Vitalik has ETH";
        actions = [
          {
            kind = "exec";
            argv = [
              "."
              (ciStepScript "mainnet-portfolio-snapshot-helios")
            ];
          }
        ];
      };
    };
  };
}
