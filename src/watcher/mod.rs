use crate::{
    cmd::helpers,
    notification::{debug, Notification},
};

use clap::ArgMatches;
use futures::StreamExt;
use std::time::Duration;
use web3::types::BlockId;

pub mod cmd;

#[tracing::instrument(name = "wrapped run watcher")]
async fn wrapped_run(args: &ArgMatches) -> Result<(), anyhow::Error> {
    let notification: &dyn Notification = &debug::DebugNotification::default();
    let address = helpers::get_address(args)?;
    let network = helpers::get_network(args)?;

    let url = match network.node_url() {
        Some(url) => url,
        None => return Err(anyhow::anyhow!("node_url_http is missing")),
    };

    let web3 = network.get_web3_client_http(url.as_str())?;

    let block_number = web3.eth().block_number().await.unwrap();

    let filter = web3.eth_filter().create_blocks_filter().await.unwrap();

    let logs_stream = filter.stream(Duration::from_secs(1));
    futures::pin_mut!(logs_stream);

    loop {
        // Process new blocks as they arrive
        let block_hash = logs_stream.next().await.unwrap().unwrap();

        // Get the block number and retrieve the block
        let block_id = BlockId::Hash(block_hash);
        let block = web3.eth().block(block_id).await.unwrap().unwrap();

        // Filter the transactions in the block for those related to the address
        let transactions = block.transactions;

        for tx_hash in transactions {
            let txn = web3
                .eth()
                .transaction(web3::types::TransactionId::Hash(tx_hash))
                .await
                .unwrap()
                .unwrap();

            match (txn.to, txn.from) {
                (Some(to), Some(from)) if to == address || from == address => {
                    // TODO: fix this notification to notify_all(notifications)
                    // TODO: fix this format to better fit any kind of txn
                    notification.notify(format!("MFM [watcher]: matches the filter of address ({}) in the transaction hash ({}), found with from ({}) and to ({}), value of {:.18} and gas_price of {}.\n", address, tx_hash, from, to, txn.value, txn.gas_price.unwrap())).unwrap();
                }
                _ => continue,
            }
        }
    }

    Ok(())
}

#[tracing::instrument(name = "run watcher")]
async fn run(args: &ArgMatches) -> Result<(), anyhow::Error> {
    wrapped_run(args).await
}
