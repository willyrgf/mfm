use std::future::Future;

use tokio::task::{JoinError, JoinSet};

use crate::error::PublishDocsError;

/// Runs async work over owned inputs while preserving input order and bounding concurrency.
pub(crate) async fn observe_many_ordered<I, O, F, Fut>(
    inputs: Vec<I>,
    max_in_flight: usize,
    f: F,
) -> Result<Vec<O>, PublishDocsError>
where
    I: Clone + Send + 'static,
    O: Send + 'static,
    F: Fn(I) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<O, PublishDocsError>> + Send + 'static,
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
            let (index, output) = result.map_err(remote_join_error)?;
            if index >= outputs.len() {
                return Err(remote_observer_error(format!(
                    "remote observation task returned out-of-range output index {index}"
                )));
            }
            if outputs[index].is_some() {
                return Err(remote_observer_error(format!(
                    "remote observation task returned duplicate output index {index}"
                )));
            }
            outputs[index] = Some(output?);
        }
    }

    outputs
        .into_iter()
        .enumerate()
        .map(|(index, output)| {
            output.ok_or_else(|| {
                remote_observer_error(format!(
                    "remote observation completed with unfilled output slot {index}"
                ))
            })
        })
        .collect()
}

pub(crate) fn remote_observer_error(message: impl Into<String>) -> PublishDocsError {
    PublishDocsError::RemoteObserver {
        message: message.into(),
    }
}

fn remote_join_error(error: JoinError) -> PublishDocsError {
    let message = if error.is_panic() {
        "remote observation task panicked"
    } else if error.is_cancelled() {
        "remote observation task was cancelled"
    } else {
        "remote observation task failed to join"
    };
    remote_observer_error(message)
}

#[cfg(test)]
mod tests {
    use super::observe_many_ordered;
    use crate::error::PublishDocsError;

    #[tokio::test]
    async fn preserves_input_order() {
        let outputs =
            observe_many_ordered(vec![3usize, 1, 2], 2, |value| async move { Ok(value * 2) })
                .await
                .expect("ordered outputs");
        assert_eq!(outputs, vec![6, 2, 4]);
    }

    #[tokio::test]
    async fn task_panic_returns_structured_error() {
        let error =
            observe_many_ordered::<usize, usize, _, _>(vec![1usize], 1, |_value| async move {
                if std::hint::black_box(true) {
                    panic!("synthetic observer panic");
                }
                Ok(0)
            })
            .await
            .expect_err("panic should become structured error");

        assert!(matches!(
            error,
            PublishDocsError::RemoteObserver { message } if message == "remote observation task panicked"
        ));
    }
}
