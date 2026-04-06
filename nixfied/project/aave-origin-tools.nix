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

  aaveV3OriginDeployTool = pkgs.writeShellScriptBin "mfm-aave-v3-origin-deploy" ''
    set -euo pipefail

    if [ -z "''${MFM_AAVE_V3_PARITY_DEPLOY_SIGNING_KEY-}" ]; then
      echo "MFM_AAVE_V3_PARITY_DEPLOY_SIGNING_KEY is required" >&2
      exit 1
    fi
    if [ -z "''${MFM_EVM_RPC_URL-}" ]; then
      echo "MFM_EVM_RPC_URL is required" >&2
      exit 1
    fi

    export MFM_AAVE_V3_PARITY_DEPLOYER_PRIVATE_KEY="$MFM_AAVE_V3_PARITY_DEPLOY_SIGNING_KEY"
    export MFM_AAVE_V3_ORIGIN_SUPPLIER="''${MFM_AAVE_V3_ORIGIN_SUPPLIER:-0x70997970C51812dc3A010C7d01b50e0d17dc79C8}"
    export MFM_AAVE_V3_ORIGIN_BORROWER="''${MFM_AAVE_V3_ORIGIN_BORROWER:-0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC}"
    export MFM_AAVE_V3_ORIGIN_USDC_SUPPLY_AMOUNT="''${MFM_AAVE_V3_ORIGIN_USDC_SUPPLY_AMOUNT:-1000000000000}"
    export MFM_AAVE_V3_ORIGIN_WBTC_COLLATERAL_AMOUNT="''${MFM_AAVE_V3_ORIGIN_WBTC_COLLATERAL_AMOUNT:-1000000000}"

    compile_manifest="$(${pkgs.coreutils}/bin/mktemp)"
    tmp="$(${pkgs.coreutils}/bin/mktemp -d)"
    cleanup() {
      ${pkgs.coreutils}/bin/rm -f "$compile_manifest"
      ${pkgs.coreutils}/bin/rm -rf "$tmp"
    }
    trap cleanup EXIT

    ${pkgs.coreutils}/bin/cat > "$compile_manifest"
    if [ ! -s "$compile_manifest" ]; then
      echo "compile manifest stdin was empty" >&2
      exit 1
    fi
    if ! ${pkgs.jq}/bin/jq -e '.kind == "evm_contract_set_compile_manifest_v1"' "$compile_manifest" >/dev/null; then
      echo "compile manifest kind must be evm_contract_set_compile_manifest_v1" >&2
      exit 1
    fi

    artifact_for_id() {
      local contract_id="$1"
      ${pkgs.jq}/bin/jq -c -e --arg id "$contract_id" '.contracts[] | select(.id == $id) | .artifact' "$compile_manifest"
    }

    usdc_artifact="$(artifact_for_id "usdc")"
    wbtc_artifact="$(artifact_for_id "wbtc")"
    pool_artifact="$(artifact_for_id "pool")"
    usdc_a_token_artifact="$(artifact_for_id "usdc_a_token")"
    wbtc_a_token_artifact="$(artifact_for_id "wbtc_a_token")"
    usdc_variable_debt_artifact="$(artifact_for_id "usdc_variable_debt_token")"
    wbtc_variable_debt_artifact="$(artifact_for_id "wbtc_variable_debt_token")"

    ${pkgs.coreutils}/bin/mkdir -p "$tmp/origin"
    ${pkgs.coreutils}/bin/cp -R "${aaveV3OriginSource}/." "$tmp/origin/"
    ${pkgs.coreutils}/bin/chmod -R u+w "$tmp/origin"
    ${pkgs.coreutils}/bin/cp ${./aave-origin-deploy/Create2Utils.sol} \
      "$tmp/origin/src/deployments/contracts/utilities/Create2Utils.sol"
    ${pkgs.coreutils}/bin/cp ${./aave-origin-deploy/MfmOriginDeploy.s.sol} \
      "$tmp/origin/scripts/MfmOriginDeploy.s.sol"

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

    (
      cd "$tmp/origin"
      ${pkgs.foundry}/bin/forge script scripts/MfmOriginDeploy.s.sol:MfmOriginDeploy \
        --rpc-url "$MFM_EVM_RPC_URL" \
        --broadcast \
        > /dev/null
    )

    deploy_report="$tmp/origin/reports/mfm-origin-deploy.json"
    if [ ! -f "$deploy_report" ]; then
      echo "missing deploy report: $deploy_report" >&2
      exit 1
    fi

    deploy_json="$(${pkgs.jq}/bin/jq -c . "$deploy_report")"

    ${pkgs.jq}/bin/jq -n -c \
      --argjson deployment "$deploy_json" \
      --argjson usdc_artifact "$usdc_artifact" \
      --argjson wbtc_artifact "$wbtc_artifact" \
      --argjson pool_artifact "$pool_artifact" \
      --argjson usdc_a_token_artifact "$usdc_a_token_artifact" \
      --argjson wbtc_a_token_artifact "$wbtc_a_token_artifact" \
      --argjson usdc_variable_debt_artifact "$usdc_variable_debt_artifact" \
      --argjson wbtc_variable_debt_artifact "$wbtc_variable_debt_artifact" \
      '{
        kind: "evm_contract_set_deploy_manifest_v1",
        contracts: [
          {
            id: "pool",
            address: $deployment.pool,
            deploy_tx_hash: "0x0",
            deploy_receipt: {},
            artifact: $pool_artifact
          },
          {
            id: "usdc",
            address: $deployment.usdc,
            deploy_tx_hash: "0x0",
            deploy_receipt: {},
            artifact: $usdc_artifact
          },
          {
            id: "wbtc",
            address: $deployment.wbtc,
            deploy_tx_hash: "0x0",
            deploy_receipt: {},
            artifact: $wbtc_artifact
          },
          {
            id: "usdc_a_token",
            address: $deployment.usdcAToken,
            deploy_tx_hash: "0x0",
            deploy_receipt: {},
            artifact: $usdc_a_token_artifact
          },
          {
            id: "wbtc_a_token",
            address: $deployment.wbtcAToken,
            deploy_tx_hash: "0x0",
            deploy_receipt: {},
            artifact: $wbtc_a_token_artifact
          },
          {
            id: "usdc_variable_debt_token",
            address: $deployment.usdcVariableDebtToken,
            deploy_tx_hash: "0x0",
            deploy_receipt: {},
            artifact: $usdc_variable_debt_artifact
          },
          {
            id: "wbtc_variable_debt_token",
            address: $deployment.wbtcVariableDebtToken,
            deploy_tx_hash: "0x0",
            deploy_receipt: {},
            artifact: $wbtc_variable_debt_artifact
          }
        ]
      }'
  '';
}
