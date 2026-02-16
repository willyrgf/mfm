{
  pkgs,
  project,
  lib,
  slots ? null,
  hooks ? null,
  postgres ? null,
  nginx ? null,
  minio ? null,
  reth ? null,
  helios ? null,
  supervisor ? null,
  ephemeral ? null,
}:

{
  # User-owned extension point.
  #
  # This directory is intended for customization that should survive framework
  # upgrades. Prefer nixfied/project/ for normal command/config wiring, and use
  # nixfied/local/ for extra apps/packages that shouldn't live in the framework.
  #
  # Notes:
  # - Apps must satisfy the Nixfied app API contract (meta.nixfied.api).
  # - Use `lib.appApi.mkNixfiedApp { ... }` to build compliant apps.
  apps =
    let
      # v2 shell-app contract inventory (project-local):
      # - evm-contract-artifact-*: json, outputs=json, wraps Foundry helper tools, failure map owner=local/default.nix
      # - mfm::keystore::* and mfm::run::*: json, outputs=json, strict typed args/env, failure map owner=local/default.nix
      failureCodesScript = {
        generic = 1;
        usage = 2;
        precondition = 3;
        unavailable = 4;
        timeout = 5;
      };
      failureCodesCargo = failureCodesScript // {
        cargoFailure = 101;
      };
      failureCodesFoundry = failureCodesScript;

      mkFoundryArtifactApp =
        {
          name,
          summary,
          details,
          tool,
        }:
        lib.appApi.mkNixfiedApp {
          inherit name;
          env = { };
          useDeps = true;
          api = lib.appApi.mkCommandApi {
            class = "json";
            inherit
              name
              summary
              details
              ;
            usage = [ "nix run .#${name}" ];
            category = "evm";
            idempotent = true;
            failureCodes = failureCodesFoundry;
          };
          script = ''
            exec ${tool} "$@"
          '';
        };

      mkMfmCliJsonApp =
        {
          name,
          summary,
          details,
          command,
          usage ? [ "nix run .#${name}" ],
          examples ? [ ],
          args ? [ ],
          env ? [ ],
          contractArgs ? null,
          contractEnv ? null,
          category ? "mfm",
          idempotent ? false,
          failureCodes ? failureCodesCargo,
        }:
        lib.appApi.mkNixfiedApp {
          inherit name;
          env = { };
          useDeps = true;
          api = lib.appApi.mkCommandApi {
            class = "json";
            inherit
              name
              summary
              details
              usage
              examples
              args
              env
              category
              idempotent
              failureCodes
              ;
            inherit contractArgs contractEnv;
          };
          script = ''
            exec cargo run -q -p mfm --bin mfm_cli -- --output-format json ${command} "$@"
          '';
        };

      runStoreDocArgs = [
        {
          name = "--artifact-root";
          description = "Artifact store root directory (default: $MFM_ARTIFACT_ROOT or ~/.mfm/run_artifacts).";
        }
        {
          name = "--database-url";
          description = "PostgreSQL connection string for run event store (default: $DATABASE_URL).";
        }
      ];

      runStoreContractArgs = [
        (lib.appApi.arg.option {
          name = "artifact-root";
          long = "--artifact-root";
          type = "string";
          required = false;
        })
        (lib.appApi.arg.option {
          name = "database-url";
          long = "--database-url";
          type = "string";
          required = false;
        })
      ];

      runStoreDocEnv = [
        {
          name = "DATABASE_URL";
          description = "Default PostgreSQL connection string used by run commands when --database-url is not provided.";
        }
        {
          name = "MFM_ARTIFACT_ROOT";
          description = "Default artifact store root used by run commands when --artifact-root is not provided.";
        }
      ];

      runStoreContractEnv = [
        (lib.appApi.env.string {
          name = "DATABASE_URL";
          required = false;
        })
        (lib.appApi.env.string {
          name = "MFM_ARTIFACT_ROOT";
          required = false;
        })
      ];
    in
    {
      evm-contract-artifact-configurable-counter = mkFoundryArtifactApp {
        name = "evm-contract-artifact-configurable-counter";
        summary = "Build ConfigurableCounter artifact JSON";
        details = "Compiles contracts/src/ConfigurableCounter.sol with Foundry and prints compact JSON {artifact:{abi,bytecode.object}} to stdout.";
        tool = "mfm-contract-artifact-configurable-counter";
      };

      evm-contract-artifact-mock-erc20 = mkFoundryArtifactApp {
        name = "evm-contract-artifact-mock-erc20";
        summary = "Build MockERC20 artifact JSON";
        details = "Compiles contracts/src/MockERC20.sol with Foundry and prints compact JSON {artifact:{abi,bytecode.object}} to stdout.";
        tool = "mfm-contract-artifact-mock-erc20";
      };

      "mfm::keystore::import" = mkMfmCliJsonApp {
        name = "mfm::keystore::import";
        summary = "Import a private key or mnemonic into keystore";
        details = "Typed wrapper over `mfm_cli keystore import` with fixed JSON output.";
        usage = [ "nix run .#mfm::keystore::import -- --import-type <privatekey|mnemonic>" ];
        category = "keystore";
        args = [
          {
            name = "--import-type";
            description = "Import type: privatekey or mnemonic (required).";
          }
          {
            name = "--label";
            description = "Optional human-readable key label.";
          }
          {
            name = "--derivation-path";
            description = "Optional derivation path for mnemonic imports.";
          }
          {
            name = "--passphrase";
            description = "Optional BIP39 passphrase.";
          }
          {
            name = "--keystore";
            description = "Optional keystore file path.";
          }
          {
            name = "--interactive";
            description = "Enable interactive input mode.";
          }
          {
            name = "--stdin";
            description = "Read key material from stdin.";
          }
        ];
        contractArgs = [
          (lib.appApi.arg.option {
            name = "import-type";
            long = "--import-type";
            short = "-t";
            type = "enum";
            values = [
              "privatekey"
              "mnemonic"
            ];
            required = true;
          })
          (lib.appApi.arg.option {
            name = "label";
            long = "--label";
            short = "-l";
            type = "string";
            required = false;
          })
          (lib.appApi.arg.option {
            name = "derivation-path";
            long = "--derivation-path";
            short = "-p";
            type = "string";
            required = false;
          })
          (lib.appApi.arg.option {
            name = "passphrase";
            long = "--passphrase";
            type = "string";
            required = false;
          })
          (lib.appApi.arg.option {
            name = "keystore";
            long = "--keystore";
            type = "string";
            required = false;
          })
          (lib.appApi.arg.flag {
            name = "interactive";
            long = "--interactive";
            short = "-i";
            required = false;
          })
          (lib.appApi.arg.flag {
            name = "stdin";
            long = "--stdin";
            required = false;
          })
        ];
        idempotent = false;
        command = "keystore import";
      };

      "mfm::keystore::list" = mkMfmCliJsonApp {
        name = "mfm::keystore::list";
        summary = "List keys in keystore";
        details = "Typed wrapper over `mfm_cli keystore list` with fixed JSON output.";
        usage = [ "nix run .#mfm::keystore::list" ];
        category = "keystore";
        args = [
          {
            name = "--keystore";
            description = "Optional keystore file path.";
          }
          {
            name = "--show-addresses";
            description = "Optional boolean toggle to include addresses.";
          }
          {
            name = "--filter-label";
            description = "Optional regex filter for key labels.";
          }
          {
            name = "--sort-by";
            description = "Sort by label|created|type.";
          }
        ];
        contractArgs = [
          (lib.appApi.arg.option {
            name = "keystore";
            long = "--keystore";
            type = "string";
            required = false;
          })
          (lib.appApi.arg.option {
            name = "show-addresses";
            long = "--show-addresses";
            type = "bool";
            required = false;
          })
          (lib.appApi.arg.option {
            name = "filter-label";
            long = "--filter-label";
            type = "string";
            required = false;
          })
          (lib.appApi.arg.option {
            name = "sort-by";
            long = "--sort-by";
            type = "enum";
            values = [
              "label"
              "created"
              "type"
            ];
            required = false;
          })
        ];
        idempotent = true;
        command = "keystore list";
      };

      "mfm::keystore::delete" = mkMfmCliJsonApp {
        name = "mfm::keystore::delete";
        summary = "Delete a key from keystore";
        details = "Typed wrapper over `mfm_cli keystore delete` with fixed JSON output.";
        usage = [ "nix run .#mfm::keystore::delete -- <ID>|--by-label <LABEL>" ];
        category = "keystore";
        args = [
          {
            name = "id";
            description = "Optional key UUID; required when --by-label is absent.";
          }
          {
            name = "--keystore";
            description = "Optional keystore file path.";
          }
          {
            name = "--yes";
            description = "Skip confirmation prompt.";
          }
          {
            name = "--by-label";
            description = "Delete by label instead of UUID.";
          }
        ];
        contractArgs = [
          (lib.appApi.arg.positional {
            name = "id";
            type = "string";
            required = false;
          })
          (lib.appApi.arg.option {
            name = "keystore";
            long = "--keystore";
            type = "string";
            required = false;
          })
          (lib.appApi.arg.flag {
            name = "yes";
            long = "--yes";
            short = "-y";
            required = false;
          })
          (lib.appApi.arg.option {
            name = "by-label";
            long = "--by-label";
            type = "string";
            required = false;
          })
        ];
        idempotent = false;
        command = "keystore delete";
      };

      "mfm::keystore::tx-sign" = mkMfmCliJsonApp {
        name = "mfm::keystore::tx-sign";
        summary = "Sign an EIP-1559 transaction";
        details = "Typed wrapper over `mfm_cli keystore tx-sign` with fixed JSON output.";
        usage = [
          "nix run .#mfm::keystore::tx-sign -- --to <ADDR> --value-wei <VALUE> --chain-id <ID> --nonce <N> --max-fee-per-gas <FEE> --max-priority-fee-per-gas <FEE> --gas-limit <N> --out <PATH>"
        ];
        category = "keystore";
        args = [
          {
            name = "--id";
            description = "Optional key UUID selector.";
          }
          {
            name = "--by-label";
            description = "Optional key label selector.";
          }
          {
            name = "--to";
            description = "Destination address (required).";
          }
          {
            name = "--value-wei";
            description = "Transfer value in wei (required).";
          }
          {
            name = "--chain-id";
            description = "EVM chain id (required).";
          }
          {
            name = "--nonce";
            description = "Transaction nonce (required).";
          }
          {
            name = "--max-fee-per-gas";
            description = "Max fee per gas (required).";
          }
          {
            name = "--max-priority-fee-per-gas";
            description = "Max priority fee per gas (required).";
          }
          {
            name = "--gas-limit";
            description = "Gas limit (required).";
          }
          {
            name = "--out";
            description = "Output path for signed raw tx (required).";
          }
          {
            name = "--data";
            description = "Optional calldata (defaults to 0x).";
          }
          {
            name = "--keystore";
            description = "Optional keystore file path.";
          }
        ];
        contractArgs = [
          (lib.appApi.arg.option {
            name = "id";
            long = "--id";
            type = "string";
            required = false;
          })
          (lib.appApi.arg.option {
            name = "by-label";
            long = "--by-label";
            type = "string";
            required = false;
          })
          (lib.appApi.arg.option {
            name = "to";
            long = "--to";
            type = "string";
            required = true;
          })
          (lib.appApi.arg.option {
            name = "value-wei";
            long = "--value-wei";
            type = "string";
            required = true;
          })
          (lib.appApi.arg.option {
            name = "chain-id";
            long = "--chain-id";
            type = "int";
            required = true;
            min = 0;
          })
          (lib.appApi.arg.option {
            name = "nonce";
            long = "--nonce";
            type = "int";
            required = true;
            min = 0;
          })
          (lib.appApi.arg.option {
            name = "max-fee-per-gas";
            long = "--max-fee-per-gas";
            type = "string";
            required = true;
          })
          (lib.appApi.arg.option {
            name = "max-priority-fee-per-gas";
            long = "--max-priority-fee-per-gas";
            type = "string";
            required = true;
          })
          (lib.appApi.arg.option {
            name = "gas-limit";
            long = "--gas-limit";
            type = "int";
            required = true;
            min = 1;
          })
          (lib.appApi.arg.option {
            name = "out";
            long = "--out";
            type = "string";
            required = true;
          })
          (lib.appApi.arg.option {
            name = "data";
            long = "--data";
            type = "string";
            required = false;
          })
          (lib.appApi.arg.option {
            name = "keystore";
            long = "--keystore";
            type = "string";
            required = false;
          })
        ];
        idempotent = false;
        command = "keystore tx-sign";
      };

      "mfm::keystore::tx-send-raw" = mkMfmCliJsonApp {
        name = "mfm::keystore::tx-send-raw";
        summary = "Submit a signed raw transaction";
        details = "Typed wrapper over `mfm_cli keystore tx-send-raw` with fixed JSON output.";
        usage = [ "nix run .#mfm::keystore::tx-send-raw -- --in <PATH>" ];
        category = "keystore";
        args = [
          {
            name = "--rpc-url";
            description = "Optional RPC URL (falls back to MFM_EVM_RPC_URL).";
          }
          {
            name = "--in";
            description = "Path to input file containing 0x-prefixed raw tx (required).";
          }
        ];
        env = [
          {
            name = "MFM_EVM_RPC_URL";
            description = "Default RPC URL used when --rpc-url is not provided.";
          }
        ];
        contractArgs = [
          (lib.appApi.arg.option {
            name = "rpc-url";
            long = "--rpc-url";
            type = "string";
            required = false;
          })
          (lib.appApi.arg.option {
            name = "in";
            long = "--in";
            type = "string";
            required = true;
          })
        ];
        contractEnv = [
          (lib.appApi.env.string {
            name = "MFM_EVM_RPC_URL";
            required = false;
          })
        ];
        idempotent = false;
        command = "keystore tx-send-raw";
      };

      "mfm::run::start" = mkMfmCliJsonApp {
        name = "mfm::run::start";
        summary = "Start a run";
        details = "Typed wrapper over `mfm_cli run start` with fixed JSON output.";
        usage = [ "nix run .#mfm::run::start -- --op-id proof --op-version v1 --op-config-json '{}'" ];
        category = "run";
        args = [
          {
            name = "--op-id";
            description = "Operation id (default: proof).";
          }
          {
            name = "--op-version";
            description = "Operation version (default: v1).";
          }
          {
            name = "--op-config-json";
            description = "Operation config JSON payload.";
          }
        ]
        ++ runStoreDocArgs;
        env = runStoreDocEnv;
        contractArgs = [
          (lib.appApi.arg.option {
            name = "op-id";
            long = "--op-id";
            type = "string";
            required = false;
          })
          (lib.appApi.arg.option {
            name = "op-version";
            long = "--op-version";
            type = "string";
            required = false;
          })
          (lib.appApi.arg.option {
            name = "op-config-json";
            long = "--op-config-json";
            type = "json";
            required = false;
          })
        ]
        ++ runStoreContractArgs;
        contractEnv = runStoreContractEnv;
        idempotent = false;
        command = "run start";
      };

      "mfm::run::resume" = mkMfmCliJsonApp {
        name = "mfm::run::resume";
        summary = "Resume a run";
        details = "Typed wrapper over `mfm_cli run resume` with fixed JSON output.";
        usage = [ "nix run .#mfm::run::resume -- <RUN_ID>" ];
        category = "run";
        args = [
          {
            name = "run-id";
            description = "Run id (UUID).";
          }
        ]
        ++ runStoreDocArgs;
        env = runStoreDocEnv;
        contractArgs = [
          (lib.appApi.arg.positional {
            name = "run-id";
            type = "string";
            required = true;
          })
        ]
        ++ runStoreContractArgs;
        contractEnv = runStoreContractEnv;
        idempotent = false;
        command = "run resume";
      };

      "mfm::run::status" = mkMfmCliJsonApp {
        name = "mfm::run::status";
        summary = "Get run status";
        details = "Typed wrapper over `mfm_cli run status` with fixed JSON output.";
        usage = [ "nix run .#mfm::run::status -- <RUN_ID>" ];
        category = "run";
        args = [
          {
            name = "run-id";
            description = "Run id (UUID).";
          }
        ]
        ++ runStoreDocArgs;
        env = runStoreDocEnv;
        contractArgs = [
          (lib.appApi.arg.positional {
            name = "run-id";
            type = "string";
            required = true;
          })
        ]
        ++ runStoreContractArgs;
        contractEnv = runStoreContractEnv;
        idempotent = true;
        command = "run status";
      };

      "mfm::run::events" = mkMfmCliJsonApp {
        name = "mfm::run::events";
        summary = "Get run events";
        details = "Typed wrapper over `mfm_cli run events` with fixed JSON output.";
        usage = [ "nix run .#mfm::run::events -- <RUN_ID> --from-seq 1" ];
        category = "run";
        args = [
          {
            name = "run-id";
            description = "Run id (UUID).";
          }
          {
            name = "--from-seq";
            description = "First sequence number to read (default: 1).";
          }
          {
            name = "--to-seq";
            description = "Optional last sequence number to read (inclusive).";
          }
        ]
        ++ runStoreDocArgs;
        env = runStoreDocEnv;
        contractArgs = [
          (lib.appApi.arg.positional {
            name = "run-id";
            type = "string";
            required = true;
          })
          (lib.appApi.arg.option {
            name = "from-seq";
            long = "--from-seq";
            type = "int";
            required = false;
            min = 1;
          })
          (lib.appApi.arg.option {
            name = "to-seq";
            long = "--to-seq";
            type = "int";
            required = false;
            min = 1;
          })
        ]
        ++ runStoreContractArgs;
        contractEnv = runStoreContractEnv;
        idempotent = true;
        command = "run events";
      };

      "mfm::run::artifacts::get" = mkMfmCliJsonApp {
        name = "mfm::run::artifacts::get";
        summary = "Get run artifact";
        details = "Typed wrapper over `mfm_cli run artifacts get` with fixed JSON output.";
        usage = [ "nix run .#mfm::run::artifacts::get -- <ARTIFACT_ID>" ];
        category = "run";
        args = [
          {
            name = "artifact-id";
            description = "Artifact id (content hash).";
          }
        ]
        ++ runStoreDocArgs;
        env = runStoreDocEnv;
        contractArgs = [
          (lib.appApi.arg.positional {
            name = "artifact-id";
            type = "string";
            required = true;
          })
        ]
        ++ runStoreContractArgs;
        contractEnv = runStoreContractEnv;
        idempotent = true;
        command = "run artifacts get";
      };

      "mfm::run::pipeline::start" = mkMfmCliJsonApp {
        name = "mfm::run::pipeline::start";
        summary = "Start an explicit pipeline";
        details = "Typed wrapper over `mfm_cli run pipeline start` with fixed JSON output.";
        usage = [ "nix run .#mfm::run::pipeline::start -- --pipeline-json '<JSON>'" ];
        category = "run";
        args = [
          {
            name = "--pipeline-json";
            description = "Pipeline JSON payload (required).";
          }
          {
            name = "--input-json";
            description = "Optional pipeline input JSON payload (default: {}).";
          }
        ]
        ++ runStoreDocArgs;
        env = runStoreDocEnv;
        contractArgs = [
          (lib.appApi.arg.option {
            name = "pipeline-json";
            long = "--pipeline-json";
            type = "json";
            required = true;
          })
          (lib.appApi.arg.option {
            name = "input-json";
            long = "--input-json";
            type = "json";
            required = false;
          })
        ]
        ++ runStoreContractArgs;
        contractEnv = runStoreContractEnv;
        idempotent = false;
        command = "run pipeline start";
      };

      "mfm::run::pipeline::deploy-configure-validate" = mkMfmCliJsonApp {
        name = "mfm::run::pipeline::deploy-configure-validate";
        summary = "Start deploy/configure/validate pipeline";
        details = "Typed wrapper over `mfm_cli run pipeline deploy-configure-validate` with fixed JSON output.";
        usage = [ "nix run .#mfm::run::pipeline::deploy-configure-validate -- --spec-json '<JSON>'" ];
        category = "run";
        args = [
          {
            name = "--spec-json";
            description = "Optional compact JSON pipeline spec.";
          }
          {
            name = "--spec-file";
            description = "Optional path to JSON pipeline spec file.";
          }
        ]
        ++ runStoreDocArgs;
        env = runStoreDocEnv;
        contractArgs = [
          (lib.appApi.arg.option {
            name = "spec-json";
            long = "--spec-json";
            type = "json";
            required = false;
          })
          (lib.appApi.arg.option {
            name = "spec-file";
            long = "--spec-file";
            type = "string";
            required = false;
          })
        ]
        ++ runStoreContractArgs;
        contractEnv = runStoreContractEnv;
        idempotent = false;
        command = "run pipeline deploy-configure-validate";
      };
    };

  # Extra flake packages (merged into `packages` output).
  packages = { };

  # Optional: extend dev shells (merged into `devShells` output).
  devShells = { };
}
