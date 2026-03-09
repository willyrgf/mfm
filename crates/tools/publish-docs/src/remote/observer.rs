use std::future::Future;

use tokio::task::JoinSet;

/// Runs async work over owned inputs while preserving input order and bounding concurrency.
pub async fn observe_many_ordered<I, O, F, Fut>(
    inputs: Vec<I>,
    max_in_flight: usize,
    f: F,
) -> Vec<O>
where
    I: Clone + Send + 'static,
    O: Send + 'static,
    F: Fn(I) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = O> + Send + 'static,
{
    let mut tasks = JoinSet::new();
    let max_in_flight = max_in_flight.max(1);
    let mut next_index = 0usize;
    let mut outputs = (0..inputs.len()).map(|_| None).collect::<Vec<Option<O>>>();

    while next_index < inputs.len() || !tasks.is_empty() {
        while next_index < inputs.len() && tasks.len() < max_in_flight {
            let index = next_index;
            let input = inputs[index].clone();
            let f = f.clone();
            tasks.spawn(async move { (index, f(input).await) });
            next_index += 1;
        }
        if let Some(result) = tasks.join_next().await {
            let (index, output) = result.expect("remote observation task panicked");
            outputs[index] = Some(output);
        }
    }

    outputs
        .into_iter()
        .map(|output| output.expect("all output slots filled"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::observe_many_ordered;

    #[tokio::test]
    async fn preserves_input_order() {
        let outputs =
            observe_many_ordered(vec![3usize, 1, 2], 2, |value| async move { value * 2 }).await;
        assert_eq!(outputs, vec![6, 2, 4]);
    }
}
