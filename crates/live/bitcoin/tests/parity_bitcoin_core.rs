use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use bitcoin::{Address, Network, ScriptBuf};
use mfm_bitcoin::{
    BitcoinNetworkId, BitcoinNetworkTag, BitcoinRoutingGenerationRef, BitcoinScanRequest,
    BitcoinSourceBinding, BitcoinSourceIdentity,
};
use mfm_bitcoin_live::transport::{
    BitcoinRpcAuthentication, BitcoinRpcEndpoint, BitcoinRpcError, BitcoinRpcSession,
};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, SchemaId};

const EXPECTED_BITCOIN_CORE_VERSION: &str = "31.0";
const EXPECTED_BITCOIN_CLI_VERSION: &str = "Bitcoin Core RPC client version v31.0.0";
const INITIAL_BLOCK_COUNT: u64 = 100;
const EXPECTED_ANCHOR_HEIGHT: u64 = INITIAL_BLOCK_COUNT + 1;
const REGTEST_BLOCK_SUBSIDY_SATS: u64 = 50 * 100_000_000;
const MAINNET_PROBE_ADDRESS: &str = "1BoatSLRHtKNngkdXEeobR76b53LETtpyT";

struct ParityConfig {
    version: String,
    host: String,
    port: String,
    cookie_file: PathBuf,
}

impl ParityConfig {
    fn from_env() -> Self {
        Self {
            version: required_env("MFM_BITCOIN_CORE_VERSION"),
            host: required_env("MFM_BITCOIN_PARITY_RPC_HOST"),
            port: required_env("MFM_BITCOIN_PARITY_RPC_PORT"),
            cookie_file: PathBuf::from(required_env("MFM_BITCOIN_PARITY_COOKIE_FILE")),
        }
    }

    fn endpoint(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }

    fn credentials(&self) -> (String, String) {
        let cookie = std::fs::read_to_string(&self.cookie_file)
            .unwrap_or_else(|_| panic!("managed Bitcoin Core cookie was unavailable"));
        let (username, password) = cookie
            .trim()
            .split_once(':')
            .unwrap_or_else(|| panic!("managed Bitcoin Core cookie was malformed"));
        (username.to_owned(), password.to_owned())
    }
}

fn required_env(name: &'static str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing managed Bitcoin Core parity input {name}"))
}

fn bitcoin_cli(config: &ParityConfig, arguments: &[&str]) -> String {
    let output = Command::new("bitcoin-cli")
        .arg("-regtest")
        .arg(format!("-rpcconnect={}", config.host))
        .arg(format!("-rpcport={}", config.port))
        .arg(format!("-rpccookiefile={}", config.cookie_file.display()))
        .args(arguments)
        .output()
        .unwrap_or_else(|_| panic!("managed bitcoin-cli was unavailable"));
    assert!(
        output.status.success(),
        "managed Bitcoin Core fixture command failed"
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|_| panic!("managed bitcoin-cli returned non-UTF-8 output"))
}

fn bitcoin_cli_version() -> String {
    let output = Command::new("bitcoin-cli")
        .arg("-version")
        .output()
        .unwrap_or_else(|_| panic!("managed bitcoin-cli was unavailable"));
    assert!(
        output.status.success(),
        "managed bitcoin-cli version failed"
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|_| panic!("managed bitcoin-cli version was not UTF-8"))
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned()
}

fn deterministic_regtest_addresses() -> (String, String, String) {
    let mining_script = ScriptBuf::from_bytes(vec![0x51]);
    let funded_script = ScriptBuf::from_bytes(vec![0x52]);
    let zero_script = ScriptBuf::from_bytes(vec![0x53]);
    (
        Address::p2wsh(&mining_script, Network::Regtest).to_string(),
        Address::p2wsh(&funded_script, Network::Regtest).to_string(),
        Address::p2wsh(&zero_script, Network::Regtest).to_string(),
    )
}

fn regtest_binding() -> BitcoinSourceBinding {
    BitcoinSourceBinding::new(
        BitcoinNetworkId::new("bitcoin-regtest").expect("network identity"),
        BitcoinNetworkTag::Regtest,
        BitcoinSourceIdentity::new("nixfied-bitcoin-core").expect("source identity"),
    )
}

fn routing_generation() -> BitcoinRoutingGenerationRef {
    let schema = SchemaId::new(
        "mfm.bitcoin.routing-generation",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x41; 32]),
    )
    .expect("schema");
    let digest = ContentDigest::from_digest(
        DigestAlgorithm::Sha256V1,
        DigestBytes::from_array([0x42; 32]),
    );
    BitcoinRoutingGenerationRef::from_reviewed(
        ContentRef::new(schema, digest).expect("routing generation"),
    )
}

#[test]
fn pinned_bitcoin_core_matches_the_checked_batch_transport() {
    let config = ParityConfig::from_env();
    assert_eq!(config.version, EXPECTED_BITCOIN_CORE_VERSION);
    let major = config
        .version
        .split('.')
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .expect("pinned Bitcoin Core major version");
    assert!(major >= 28);
    assert_eq!(bitcoin_cli_version(), EXPECTED_BITCOIN_CLI_VERSION);
    assert_eq!(bitcoin_cli(&config, &["getblockcount"]).trim(), "0");

    let (mining_address, funded_address, zero_address) = deterministic_regtest_addresses();
    let initial_blocks: serde_json::Value = serde_json::from_str(&bitcoin_cli(
        &config,
        &[
            "generatetoaddress",
            &INITIAL_BLOCK_COUNT.to_string(),
            &mining_address,
        ],
    ))
    .expect("initial generated block hashes");
    assert_eq!(
        initial_blocks
            .as_array()
            .expect("initial generated block hash array")
            .len(),
        INITIAL_BLOCK_COUNT as usize
    );
    let fixture_block: serde_json::Value = serde_json::from_str(&bitcoin_cli(
        &config,
        &["generatetoaddress", "1", &funded_address],
    ))
    .expect("fixture block hash");
    assert_eq!(
        fixture_block
            .as_array()
            .expect("fixture block hash array")
            .len(),
        1
    );

    let chain_info: serde_json::Value =
        serde_json::from_str(&bitcoin_cli(&config, &["getblockchaininfo"]))
            .expect("blockchain info");
    assert_eq!(chain_info["chain"], "regtest");
    assert_eq!(chain_info["initialblockdownload"], false);

    let mut addresses = vec![funded_address.clone(), zero_address.clone()];
    addresses.sort();
    let request =
        BitcoinScanRequest::new(BitcoinNetworkTag::Regtest, addresses).expect("scan request");
    let (username, password) = config.credentials();
    let authentication =
        BitcoinRpcAuthentication::new(username.clone(), zeroize::Zeroizing::new(password.clone()))
            .expect("resolved fixture authentication");
    let session = BitcoinRpcSession::new(
        BitcoinRpcEndpoint::new(config.endpoint()).expect("endpoint"),
        Some(authentication),
        regtest_binding(),
        routing_generation(),
        Duration::from_secs(30),
    )
    .expect("checked parity session");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("parity runtime");
    let info = runtime
        .block_on(session.get_blockchain_info())
        .expect("blockchain info");
    assert_eq!(info.chain(), "regtest");
    assert!(!info.initial_block_download());
    let response = runtime
        .block_on(session.scan_tx_out_set_start(&request))
        .expect("live scan");
    let confirmation = runtime
        .block_on(session.get_block_hash(response.height()))
        .expect("block hash");

    assert_eq!(response.height(), EXPECTED_ANCHOR_HEIGHT);
    assert_eq!(response.anchor_hash(), confirmation);
    let balances = response
        .balances()
        .iter()
        .map(|balance| (balance.address(), balance.balance_sats()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        balances.get(funded_address.as_str()),
        Some(&REGTEST_BLOCK_SUBSIDY_SATS)
    );
    assert_eq!(balances.get(zero_address.as_str()), Some(&0));

    let mainnet_binding = BitcoinSourceBinding::new(
        BitcoinNetworkId::new("bitcoin-mainnet").expect("network identity"),
        BitcoinNetworkTag::Main,
        BitcoinSourceIdentity::new("nixfied-bitcoin-core").expect("source identity"),
    );
    let _mainnet_request = BitcoinScanRequest::new(
        BitcoinNetworkTag::Main,
        vec![MAINNET_PROBE_ADDRESS.to_owned()],
    )
    .expect("mainnet probe request");
    let mainnet_session = BitcoinRpcSession::new(
        BitcoinRpcEndpoint::new(config.endpoint()).expect("endpoint"),
        Some(
            BitcoinRpcAuthentication::new(
                username.clone(),
                zeroize::Zeroizing::new(password.clone()),
            )
            .expect("resolved fixture authentication"),
        ),
        mainnet_binding,
        routing_generation(),
        Duration::from_secs(30),
    )
    .expect("wrong-chain probe session");
    let mismatched = runtime
        .block_on(mainnet_session.get_blockchain_info())
        .expect("representable source response");
    assert_eq!(mismatched.chain(), "regtest");

    let rejected_credential = "mfm-parity-rejected-public-fixture";
    let rejected_session = BitcoinRpcSession::new(
        BitcoinRpcEndpoint::new(config.endpoint()).expect("endpoint"),
        Some(
            BitcoinRpcAuthentication::new(
                username.clone(),
                zeroize::Zeroizing::new(rejected_credential.to_owned()),
            )
            .expect("rejected fixture authentication"),
        ),
        regtest_binding(),
        routing_generation(),
        Duration::from_secs(30),
    )
    .expect("rejected-authentication session");
    let error = runtime
        .block_on(rejected_session.get_blockchain_info())
        .expect_err("Bitcoin Core must reject the wrong fixture credential");
    assert!(matches!(error, BitcoinRpcError::HttpStatus(401 | 403)));
    let rendered = format!("{error:?} {error}");
    for sensitive in [
        config.endpoint(),
        username,
        password,
        rejected_credential.to_owned(),
    ] {
        assert!(
            !rendered.contains(&sensitive),
            "transport failure exposed fixture credential material"
        );
    }
}
