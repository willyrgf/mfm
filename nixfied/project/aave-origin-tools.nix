{ pkgs }:
let
  aaveV3OriginRepoUrl = "https://github.com/aave-dao/aave-v3-origin";
  aaveV3OriginCommit = "1e3d70c4151a94166ebc59e2eaa4aff6e6ba6978";
  aaveV3OriginHash = "sha256-PNDpFxNozzot0t7XESS7YaNVaaNfpL/PKjCF2J+zJIw=";
  aaveV3OriginSource = pkgs.fetchFromGitHub {
    owner = "aave-dao";
    repo = "aave-v3-origin";
    rev = aaveV3OriginCommit;
    hash = aaveV3OriginHash;
    fetchSubmodules = true;
  };
in
rec {
  aaveV3OriginFetchTool = pkgs.writeShellScriptBin "mfm-aave-v3-origin-fetch" ''
    set -euo pipefail

    origin_source="${aaveV3OriginSource}"
    expected_repo_url="''${MFM_AAVE_V3_ORIGIN_EXPECTED_REPO_URL:-${aaveV3OriginRepoUrl}}"
    expected_commit_sha="''${MFM_AAVE_V3_ORIGIN_EXPECTED_COMMIT_SHA:-${aaveV3OriginCommit}}"

    if [ "$expected_repo_url" != "${aaveV3OriginRepoUrl}" ] || [ "$expected_commit_sha" != "${aaveV3OriginCommit}" ]; then
      echo "requested source $expected_repo_url@$expected_commit_sha is unsupported by the current Aave Origin backend" >&2
      exit 1
    fi

    ${pkgs.jq}/bin/jq -n -c \
      --arg repo_url "${aaveV3OriginRepoUrl}" \
      --arg commit_sha "${aaveV3OriginCommit}" \
      --arg local_path "$origin_source" \
      '{
        kind: "aave_v3_origin_source_v1",
        source: {
          repo_url: $repo_url,
          commit_sha: $commit_sha,
          local_path: $local_path
        }
      }'
  '';

  aaveV3OriginCompileTool = pkgs.writeShellScriptBin "mfm-aave-v3-origin-compile" ''
        set -euo pipefail

        origin_source="${aaveV3OriginSource}"
        expected_repo_url="''${MFM_AAVE_V3_ORIGIN_EXPECTED_REPO_URL:-${aaveV3OriginRepoUrl}}"
        expected_commit_sha="''${MFM_AAVE_V3_ORIGIN_EXPECTED_COMMIT_SHA:-${aaveV3OriginCommit}}"

        if [ "$expected_repo_url" != "${aaveV3OriginRepoUrl}" ] || [ "$expected_commit_sha" != "${aaveV3OriginCommit}" ]; then
          echo "requested source $expected_repo_url@$expected_commit_sha is unsupported by the current Aave Origin backend" >&2
          exit 1
        fi

        tmp="$(${pkgs.coreutils}/bin/mktemp -d)"
        cleanup() { ${pkgs.coreutils}/bin/rm -rf "$tmp"; }
        trap cleanup EXIT

        ${pkgs.coreutils}/bin/mkdir -p "$tmp/origin"
        ${pkgs.coreutils}/bin/cp -R "$origin_source/." "$tmp/origin/"
        ${pkgs.coreutils}/bin/chmod -R u+w "$tmp/origin"

        cat > "$tmp/origin/foundry.toml" <<EOF
    [profile.default]
    src = "src"
    test = "tests"
    script = "scripts"
    optimizer = true
    optimizer_runs = 200
    solc = "${pkgs.solc}/bin/solc"
    evm_version = "shanghai"
    bytecode_hash = "none"
    out = "out"
    libs = ["lib"]
    remappings = []
    fs_permissions = [
      { access = "write", path = "./reports" },
      { access = "read", path = "./out" },
      { access = "read", path = "./config" },
    ]
    ffi = true
    EOF

        (
          cd "$tmp/origin"
          ${pkgs.foundry}/bin/forge build --quiet > /dev/null
        )

        token_artifact="$tmp/origin/out/TestnetERC20.sol/TestnetERC20.json"
        pool_artifact="$tmp/origin/out/Pool.sol/Pool.json"
        a_token_artifact="$tmp/origin/out/AToken.sol/AToken.json"
        variable_debt_artifact="$tmp/origin/out/VariableDebtToken.sol/VariableDebtToken.json"

        token_abi=$(${pkgs.jq}/bin/jq -c '.abi' "$token_artifact")
        token_bytecode=$(${pkgs.jq}/bin/jq -r '.bytecode.object' "$token_artifact")
        pool_abi=$(${pkgs.jq}/bin/jq -c '.abi' "$pool_artifact")
        pool_bytecode=$(${pkgs.jq}/bin/jq -r '.bytecode.object' "$pool_artifact")
        a_token_abi=$(${pkgs.jq}/bin/jq -c '.abi' "$a_token_artifact")
        a_token_bytecode=$(${pkgs.jq}/bin/jq -r '.bytecode.object' "$a_token_artifact")
        variable_debt_abi=$(${pkgs.jq}/bin/jq -c '.abi' "$variable_debt_artifact")
        variable_debt_bytecode=$(${pkgs.jq}/bin/jq -r '.bytecode.object' "$variable_debt_artifact")

        ${pkgs.jq}/bin/jq -n -c \
          --arg repo_url "${aaveV3OriginRepoUrl}" \
          --arg commit_sha "${aaveV3OriginCommit}" \
          --argjson token_abi "$token_abi" \
          --arg token_bytecode "$token_bytecode" \
          --argjson pool_abi "$pool_abi" \
          --arg pool_bytecode "$pool_bytecode" \
          --argjson a_token_abi "$a_token_abi" \
          --arg a_token_bytecode "$a_token_bytecode" \
          --argjson variable_debt_abi "$variable_debt_abi" \
          --arg variable_debt_bytecode "$variable_debt_bytecode" \
          '{
            kind: "aave_v3_origin_compile_manifest_v1",
            source: {
              repo_url: $repo_url,
              commit_sha: $commit_sha
            },
            contracts: [
              {
                id: "usdc",
                artifact: { abi: $token_abi, bytecode: { object: $token_bytecode } },
                constructor_args: ["USDX", "USDX", 6]
              },
              {
                id: "wbtc",
                artifact: { abi: $token_abi, bytecode: { object: $token_bytecode } },
                constructor_args: ["WBTC", "WBTC", 8]
              },
              {
                id: "pool",
                artifact: { abi: $pool_abi, bytecode: { object: $pool_bytecode } },
                constructor_args: []
              },
              {
                id: "a_token",
                artifact: { abi: $a_token_abi, bytecode: { object: $a_token_bytecode } },
                constructor_args: []
              },
              {
                id: "variable_debt_token",
                artifact: { abi: $variable_debt_abi, bytecode: { object: $variable_debt_bytecode } },
                constructor_args: []
              }
            ]
          }'
  '';

  aaveV3OriginDeployTool = pkgs.writeShellScriptBin "mfm-aave-v3-origin-deploy" ''
        set -euo pipefail

        origin_source="${aaveV3OriginSource}"
        expected_repo_url="''${MFM_AAVE_V3_ORIGIN_EXPECTED_REPO_URL:-${aaveV3OriginRepoUrl}}"
        expected_commit_sha="''${MFM_AAVE_V3_ORIGIN_EXPECTED_COMMIT_SHA:-${aaveV3OriginCommit}}"

        if [ "$expected_repo_url" != "${aaveV3OriginRepoUrl}" ] || [ "$expected_commit_sha" != "${aaveV3OriginCommit}" ]; then
          echo "requested source $expected_repo_url@$expected_commit_sha is unsupported by the current Aave Origin backend" >&2
          exit 1
        fi

        deploy_signing_key="''${MFM_AAVE_V3_ORIGIN_DEPLOY_SIGNER_VALUE-}"
        deploy_signing_key_env="''${MFM_AAVE_V3_ORIGIN_DEPLOY_SIGNING_KEY_ENV:-MFM_AAVE_V3_PARITY_DEPLOY_SIGNING_KEY}"
        if [ -z "$deploy_signing_key" ]; then
          deploy_signing_key="''${!deploy_signing_key_env-}"
        fi
        if [ -z "$deploy_signing_key" ]; then
          echo "$deploy_signing_key_env is required" >&2
          exit 1
        fi
        export MFM_AAVE_V3_PARITY_DEPLOYER_PRIVATE_KEY="$deploy_signing_key"

        if [ -z "''${MFM_AAVE_V3_ORIGIN_SUPPLIER-}" ]; then
          export MFM_AAVE_V3_ORIGIN_SUPPLIER="0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
        fi
        if [ -z "''${MFM_AAVE_V3_ORIGIN_BORROWER-}" ]; then
          export MFM_AAVE_V3_ORIGIN_BORROWER="0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"
        fi
        if [ -z "''${MFM_AAVE_V3_ORIGIN_USDC_SUPPLY_AMOUNT-}" ]; then
          export MFM_AAVE_V3_ORIGIN_USDC_SUPPLY_AMOUNT="1000000000000"
        fi
        if [ -z "''${MFM_AAVE_V3_ORIGIN_WBTC_COLLATERAL_AMOUNT-}" ]; then
          export MFM_AAVE_V3_ORIGIN_WBTC_COLLATERAL_AMOUNT="1000000000"
        fi
        rpc_url="''${MFM_AAVE_V3_ORIGIN_RPC_URL-}"
        rpc_url_env="''${MFM_AAVE_V3_ORIGIN_RPC_URL_ENV:-MFM_EVM_RPC_URL}"
        if [ -z "$rpc_url" ]; then
          rpc_url="''${!rpc_url_env-}"
        fi
        if [ -z "$rpc_url" ]; then
          echo "$rpc_url_env is required" >&2
          exit 1
        fi

        tmp="$(${pkgs.coreutils}/bin/mktemp -d)"
        cleanup() { ${pkgs.coreutils}/bin/rm -rf "$tmp"; }
        trap cleanup EXIT

        ${pkgs.coreutils}/bin/mkdir -p "$tmp/origin"
        ${pkgs.coreutils}/bin/cp -R "$origin_source/." "$tmp/origin/"
        ${pkgs.coreutils}/bin/chmod -R u+w "$tmp/origin"

        cat > "$tmp/origin/foundry.toml" <<EOF
    [profile.default]
    src = "src"
    test = "tests"
    script = "scripts"
    optimizer = true
    optimizer_runs = 200
    via_ir = true
    solc = "${pkgs.solc}/bin/solc"
    evm_version = "shanghai"
    bytecode_hash = "none"
    out = "out"
    libs = ["lib"]
    remappings = []
    fs_permissions = [
      { access = "write", path = "./reports" },
      { access = "read", path = "./out" },
      { access = "read", path = "./config" },
    ]
    ffi = true
    EOF

        cat > "$tmp/origin/src/deployments/contracts/utilities/Create2Utils.sol" <<'EOF'
    // SPDX-License-Identifier: BUSL-1.1
    pragma solidity ^0.8.0;

    library Create2Utils {
      // https://github.com/safe-global/safe-singleton-factory
      address public constant CREATE2_FACTORY = 0x914d7Fec6aaC8cd542e72Bca78B30650d45643d7;

      function _create2Deploy(bytes32 salt, bytes memory bytecode) internal returns (address) {
        bytes32 initcodeHash = keccak256(abi.encodePacked(bytecode));

        if (isContractDeployed(CREATE2_FACTORY) == false) {
          address computed = computeCreate2AddressFrom(address(this), salt, initcodeHash);
          if (isContractDeployed(computed)) {
            return computed;
          }

          address deployedAt;
          assembly {
            deployedAt := create2(0, add(bytecode, 0x20), mload(bytecode), salt)
          }
          require(deployedAt != address(0), 'failure at create2 deployment');
          require(deployedAt == computed, 'failure at create2 address derivation');
          return deployedAt;
        }

        address computed = computeCreate2Address(salt, initcodeHash);
        if (isContractDeployed(computed)) {
          return computed;
        }

        bytes memory creationBytecode = abi.encodePacked(salt, bytecode);
        (bool success, bytes memory returnData) = CREATE2_FACTORY.call(creationBytecode);
        require(success, 'failure at create2 deployment');
        // forge-lint: disable-next-line(unsafe-typecast)
        address deployedAt = address(uint160(bytes20(returnData)));
        require(deployedAt == computed, 'failure at create2 address derivation');
        return deployedAt;
      }

      function isContractDeployed(address _addr) internal view returns (bool isContract) {
        return (_addr.code.length > 0);
      }

      function computeCreate2Address(bytes32 salt, bytes32 initcodeHash) internal view returns (address) {
        if (isContractDeployed(CREATE2_FACTORY)) {
          return computeCreate2AddressFrom(CREATE2_FACTORY, salt, initcodeHash);
        }
        return computeCreate2AddressFrom(address(this), salt, initcodeHash);
      }

      function computeCreate2Address(bytes32 salt, bytes memory bytecode) internal view returns (address) {
        return computeCreate2Address(salt, keccak256(abi.encodePacked(bytecode)));
      }

      function computeCreate2AddressFrom(address deployer, bytes32 salt, bytes32 initcodeHash) internal pure returns (address) {
        return addressFromLast20Bytes(keccak256(abi.encodePacked(bytes1(0xff), deployer, salt, initcodeHash)));
      }

      function addressFromLast20Bytes(bytes32 bytesValue) internal pure returns (address) {
        return address(uint160(uint256(bytesValue)));
      }
    }
    EOF

        cat > "$tmp/origin/scripts/MfmOriginDeploy.s.sol" <<'EOF'
    // SPDX-License-Identifier: BUSL-1.1
    pragma solidity ^0.8.0;

    import 'forge-std/Script.sol';
    import 'forge-std/StdJson.sol';

    import {AaveV3BatchOrchestration} from '../src/deployments/projects/aave-v3-batched/AaveV3BatchOrchestration.sol';
    import {DefaultMarketInput} from '../src/deployments/inputs/DefaultMarketInput.sol';
    import {MarketReport, Roles, MarketConfig, DeployFlags} from '../src/deployments/interfaces/IMarketReportTypes.sol';
    import {IAaveV3ConfigEngine} from '../src/contracts/extensions/v3-config-engine/AaveV3ConfigEngine.sol';
    import {AaveV3TestListing} from '../tests/mocks/AaveV3TestListing.sol';
    import {ACLManager} from '../src/contracts/protocol/configuration/ACLManager.sol';
    import {WETH9} from '../src/contracts/dependencies/weth/WETH9.sol';
    import {TestnetERC20} from '../src/contracts/mocks/testnet-helpers/TestnetERC20.sol';
    import {IPool} from '../src/contracts/interfaces/IPool.sol';
    import {DataTypes} from '../src/contracts/protocol/libraries/types/DataTypes.sol';

    contract MfmOriginDeploy is Script, DefaultMarketInput {
      using stdJson for string;

      function run() external {
        uint256 deployerPrivateKey = vm.envUint('MFM_AAVE_V3_PARITY_DEPLOYER_PRIVATE_KEY');
        address deployer = vm.addr(deployerPrivateKey);
        address supplier = vm.envAddress('MFM_AAVE_V3_ORIGIN_SUPPLIER');
        address borrower = vm.envAddress('MFM_AAVE_V3_ORIGIN_BORROWER');
        uint256 usdcSupplyAmount = vm.envUint('MFM_AAVE_V3_ORIGIN_USDC_SUPPLY_AMOUNT');
        uint256 wbtcCollateralAmount = vm.envUint('MFM_AAVE_V3_ORIGIN_WBTC_COLLATERAL_AMOUNT');

        (
          Roles memory roles,
          MarketConfig memory config,
          DeployFlags memory flags,
          MarketReport memory report
        ) = _getMarketInput(deployer);
        roles.marketOwner = deployer;
        roles.poolAdmin = deployer;
        roles.emergencyAdmin = deployer;

        vm.startBroadcast(deployerPrivateKey);

        address weth = address(new WETH9());
        config.wrappedNativeToken = weth;

        report = AaveV3BatchOrchestration.deployAaveV3(deployer, roles, config, flags, report);

        AaveV3TestListing listing = new AaveV3TestListing(
          IAaveV3ConfigEngine(report.configEngine),
          roles.poolAdmin,
          weth,
          report
        );

        ACLManager manager = ACLManager(report.aclManager);
        manager.addPoolAdmin(address(listing));
        listing.execute();

        address usdc = listing.USDX_ADDRESS();
        address wbtc = listing.WBTC_ADDRESS();

        TestnetERC20(usdc).mint(supplier, usdcSupplyAmount);
        TestnetERC20(wbtc).mint(borrower, wbtcCollateralAmount);

        DataTypes.ReserveDataLegacy memory usdcReserve = IPool(report.poolProxy).getReserveData(
          usdc
        );
        DataTypes.ReserveDataLegacy memory wbtcReserve = IPool(report.poolProxy).getReserveData(
          wbtc
        );

        vm.stopBroadcast();

        string memory root = 'mfm_origin_deploy';
        vm.serializeString(root, 'kind', 'aave_v3_origin_deploy_output_v1');
        vm.serializeAddress(root, 'pool', report.poolProxy);
        vm.serializeAddress(root, 'usdc', usdc);
        vm.serializeAddress(root, 'wbtc', wbtc);
        vm.serializeAddress(root, 'usdcAToken', usdcReserve.aTokenAddress);
        vm.serializeAddress(root, 'wbtcAToken', wbtcReserve.aTokenAddress);
        vm.serializeAddress(root, 'usdcVariableDebtToken', usdcReserve.variableDebtTokenAddress);
        string memory out = vm.serializeAddress(
          root,
          'wbtcVariableDebtToken',
          wbtcReserve.variableDebtTokenAddress
        );
        vm.writeJson(out, './reports/mfm-origin-deploy.json');
      }
    }
    EOF

        (
          cd "$tmp/origin"
          ${pkgs.foundry}/bin/forge script scripts/MfmOriginDeploy.s.sol:MfmOriginDeploy \
            --rpc-url "$rpc_url" \
            --broadcast \
            > /dev/null
        )

        deploy_report="$tmp/origin/reports/mfm-origin-deploy.json"
        if [ ! -f "$deploy_report" ]; then
          echo "missing deploy report: $deploy_report" >&2
          exit 1
        fi

        token_artifact="$tmp/origin/out/TestnetERC20.sol/TestnetERC20.json"
        pool_artifact="$tmp/origin/out/Pool.sol/Pool.json"
        a_token_artifact="$tmp/origin/out/AToken.sol/AToken.json"
        variable_debt_artifact="$tmp/origin/out/VariableDebtToken.sol/VariableDebtToken.json"

        deploy_json=$(${pkgs.jq}/bin/jq -c . "$deploy_report")
        token_abi=$(${pkgs.jq}/bin/jq -c '.abi' "$token_artifact")
        token_bytecode=$(${pkgs.jq}/bin/jq -r '.bytecode.object' "$token_artifact")
        pool_abi=$(${pkgs.jq}/bin/jq -c '.abi' "$pool_artifact")
        pool_bytecode=$(${pkgs.jq}/bin/jq -r '.bytecode.object' "$pool_artifact")
        a_token_abi=$(${pkgs.jq}/bin/jq -c '.abi' "$a_token_artifact")
        a_token_bytecode=$(${pkgs.jq}/bin/jq -r '.bytecode.object' "$a_token_artifact")
        variable_debt_abi=$(${pkgs.jq}/bin/jq -c '.abi' "$variable_debt_artifact")
        variable_debt_bytecode=$(${pkgs.jq}/bin/jq -r '.bytecode.object' "$variable_debt_artifact")

        ${pkgs.jq}/bin/jq -n -c \
          --arg repo_url "${aaveV3OriginRepoUrl}" \
          --arg commit_sha "${aaveV3OriginCommit}" \
          --argjson deployment "$deploy_json" \
          --argjson token_abi "$token_abi" \
          --arg token_bytecode "$token_bytecode" \
          --argjson pool_abi "$pool_abi" \
          --arg pool_bytecode "$pool_bytecode" \
          --argjson a_token_abi "$a_token_abi" \
          --arg a_token_bytecode "$a_token_bytecode" \
          --argjson variable_debt_abi "$variable_debt_abi" \
          --arg variable_debt_bytecode "$variable_debt_bytecode" \
          '{
            kind: "aave_v3_origin_deploy_output_v1",
            source: {
              repo_url: $repo_url,
              commit_sha: $commit_sha
            },
            deployment: $deployment,
            contracts: [
              {
                id: "pool",
                address: $deployment.pool,
                deploy_tx_hash: "0x0",
                deploy_receipt: {},
                artifact: { abi: $pool_abi, bytecode: { object: $pool_bytecode } }
              },
              {
                id: "usdc",
                address: $deployment.usdc,
                deploy_tx_hash: "0x0",
                deploy_receipt: {},
                artifact: { abi: $token_abi, bytecode: { object: $token_bytecode } }
              },
              {
                id: "wbtc",
                address: $deployment.wbtc,
                deploy_tx_hash: "0x0",
                deploy_receipt: {},
                artifact: { abi: $token_abi, bytecode: { object: $token_bytecode } }
              },
              {
                id: "usdc_a_token",
                address: $deployment.usdcAToken,
                deploy_tx_hash: "0x0",
                deploy_receipt: {},
                artifact: { abi: $a_token_abi, bytecode: { object: $a_token_bytecode } }
              },
              {
                id: "wbtc_a_token",
                address: $deployment.wbtcAToken,
                deploy_tx_hash: "0x0",
                deploy_receipt: {},
                artifact: { abi: $a_token_abi, bytecode: { object: $a_token_bytecode } }
              },
              {
                id: "usdc_variable_debt_token",
                address: $deployment.usdcVariableDebtToken,
                deploy_tx_hash: "0x0",
                deploy_receipt: {},
                artifact: { abi: $variable_debt_abi, bytecode: { object: $variable_debt_bytecode } }
              },
              {
                id: "wbtc_variable_debt_token",
                address: $deployment.wbtcVariableDebtToken,
                deploy_tx_hash: "0x0",
                deploy_receipt: {},
                artifact: { abi: $variable_debt_abi, bytecode: { object: $variable_debt_bytecode } }
              }
            ]
          }'
  '';
}
