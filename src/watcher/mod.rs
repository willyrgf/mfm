use std::time;

use crate::{cmd::helpers, config::Config};
use clap::ArgMatches;
use futures::StreamExt;
use web3::types::FilterBuilder;

pub mod cmd;

#[tracing::instrument(name = "wrapped run watcher")]
async fn wrapped_run(args: &ArgMatches) -> Result<(), anyhow::Error> {
    let config = Config::global();

    let address = helpers::get_address(args)?;
    let network = helpers::get_network(args)?;

    let client = network.get_web3_client_http();

    println!("config: {:?}", config);
    println!("address: {:?}", address);
    println!("network: {:?}", network);

    let filter = FilterBuilder::default().address(vec![address]).build();

    let filter = client
        .eth_filter()
        .create_logs_filter(filter)
        .await
        .map_err(|e| anyhow::anyhow!("failed to create a log filter, got {:?}", e))?;

    let log_stream = filter.stream(time::Duration::from_secs(1));
    futures::pin_mut!(log_stream);

    let log = log_stream.next().await.unwrap();

    println!("log: {:?}", log);

    Ok(())
}

#[tracing::instrument(name = "run watcher")]
async fn run(args: &ArgMatches) -> Result<(), anyhow::Error> {
    wrapped_run(args).await
}
