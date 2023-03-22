use crate::cmd::helpers;

use bigdecimal::Zero;
use clap::ArgMatches;
use futures::StreamExt;
use std::time::Duration;
use web3::types::FilterBuilder;

pub mod cmd;

#[tracing::instrument(name = "wrapped run watcher")]
async fn wrapped_run(args: &ArgMatches) -> Result<(), anyhow::Error> {
    let address = helpers::get_address(args)?;
    let network = helpers::get_network(args)?;

    let url = match network.node_url() {
        Some(url) => url,
        None => return Err(anyhow::anyhow!("node_url is missing")),
    };

    let web3 = network.get_web3_client_http(url.as_str()).unwrap();

    let filter = FilterBuilder::default().address(vec![address]).build();

    let filter = web3
        .eth_filter()
        .create_logs_filter(filter)
        .await
        .map_err(|e| anyhow::anyhow!("failed to create a log filter, got {:?}", e))?;

    loop {
        let logs = filter.logs().await.unwrap();
        logs.iter().for_each(|log| println!("{:?}", log));

        if !logs.len().is_zero() {
            break;
        }
    }
    // let log_stream = filter.stream(Duration::from_secs(1));
    // futures::pin_mut!(log_stream);

    // let log = log_stream.next().await.unwrap();

    // println!("log: {:?}", log);

    Ok(())
}

#[tracing::instrument(name = "run watcher")]
async fn run(args: &ArgMatches) -> Result<(), anyhow::Error> {
    wrapped_run(args).await
}
