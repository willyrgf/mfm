use std::time::{self, Duration};

use crate::{cmd::helpers, config::Config};
use clap::ArgMatches;
use futures::StreamExt;
use web3::types::{FilterBuilder, TransactionId};

pub mod cmd;

#[tracing::instrument(name = "wrapped run watcher")]
async fn wrapped_run(args: &ArgMatches) -> Result<(), anyhow::Error> {
    let config = Config::global();

    let address = helpers::get_address(args)?;
    let network = helpers::get_network(args)?;

    if network.node_url().is_some() {
        let ws_web3 = network.get_web3_client_ws().await?;

        // Build a filter for new transactions to the given address
        let filter = FilterBuilder::default().address(vec![address]).build();

        // Create a new event stream for new transactions to the given address
        let mut event_stream = ws_web3
            .eth_subscribe()
            .subscribe_logs(filter)
            .await
            .unwrap();

        // Start a loop to watch for new transactions
        loop {
            // Wait for a new event to be received
            let event_txn_hash = event_stream
                .next()
                .await
                .unwrap()
                .unwrap()
                .transaction_hash
                .unwrap();

            // Print out the details of the new transaction
            let transaction = ws_web3
                .eth()
                .transaction(TransactionId::Hash(event_txn_hash))
                .await
                .unwrap();
            println!("New transaction: {:?}", transaction);

            // Wait for 5 seconds before checking for new events again
            std::thread::sleep(Duration::from_secs(5));
        }
    }

    println!("config: {:?}", config);
    println!("address: {:?}", address);
    println!("network: {:?}", network);

    let web3 = network.get_web3_client_http();

    let filter = FilterBuilder::default().address(vec![address]).build();

    let filter = web3
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
