#[tokio::main]
async fn main() {
    if mfm_wallet_authority_provider_test_support::run_provider_from_stdio()
        .await
        .is_err()
    {
        std::process::exit(1);
    }
}
