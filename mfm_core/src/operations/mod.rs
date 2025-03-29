//! Operation framework for building composable operations with proper error handling
//!
//! This module provides a framework for building operations that can be composed together
//! and handle errors in a functional way.

use crate::contexts::portfolio::PortfolioContext;
use crate::portfolio::PortfolioError;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use thiserror::Error;

/// Result type for operations
pub type OperationResult<T> = Result<T, OperationError>;

/// Error type for operations
#[derive(Debug, Error)]
pub enum OperationError {
    #[error("Portfolio error: {0}")]
    Portfolio(PortfolioError),

    #[error("Operation failed: {0}")]
    Failed(String),

    #[error("Operation interrupted")]
    Interrupted,

    #[error("Operation timed out")]
    Timeout,
}

/// Operation tag for categorizing operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationTag {
    Pure,            // Pure operation with no side effects
    Network,         // Network operation with potential side effects
    Blockchain,      // Blockchain operation with potential side effects
    Storage,         // Storage operation with potential side effects
    UserInteraction, // Operation that interacts with the user
}

/// Operation trait for executable operations
pub trait Operation<T> {
    /// Execute the operation and return a result
    fn execute(&self) -> Pin<Box<dyn Future<Output = OperationResult<T>> + Send + '_>>;

    /// Get operation tags
    fn tags(&self) -> Vec<OperationTag> {
        vec![OperationTag::Pure]
    }

    /// Map operation result to another type
    fn map<F, U>(self, f: F) -> MapOperation<Self, F, T, U>
    where
        Self: Sized,
        F: FnOnce(T) -> U + Send + Sync + Clone + 'static,
        U: Send + 'static,
    {
        MapOperation {
            inner: self,
            f,
            _phantom: PhantomData,
        }
    }

    /// Chain operations
    fn and_then<F, O2, U>(self, f: F) -> AndThenOperation<Self, F, T, U>
    where
        Self: Sized,
        F: FnOnce(T) -> O2 + Send + Sync + Clone + 'static,
        O2: Operation<U>,
        U: Send + 'static,
    {
        AndThenOperation {
            inner: self,
            f,
            _phantom: PhantomData,
        }
    }

    /// Recover from errors
    fn or_else<F, O2>(self, f: F) -> OrElseOperation<Self, F, T>
    where
        Self: Sized,
        F: FnOnce(OperationError) -> O2 + Send + Sync + Clone + 'static,
        O2: Operation<T>,
    {
        OrElseOperation {
            inner: self,
            f,
            _phantom: PhantomData,
        }
    }
}

/// Map operation for mapping the result of an operation
pub struct MapOperation<O, F, T, U> {
    inner: O,
    f: F,
    _phantom: PhantomData<(F, T, U)>,
}

impl<O, F, T, U> Operation<U> for MapOperation<O, F, T, U>
where
    O: Operation<T>,
    F: FnOnce(T) -> U + Send + Sync + Clone + 'static,
    T: Send + 'static,
    U: Send + 'static,
{
    fn execute(&self) -> Pin<Box<dyn Future<Output = OperationResult<U>> + Send + '_>> {
        let inner_future = self.inner.execute();
        let f = self.f.clone();

        Box::pin(async move {
            let inner_result = inner_future.await?;
            Ok(f(inner_result))
        })
    }

    fn tags(&self) -> Vec<OperationTag> {
        self.inner.tags()
    }
}

/// AndThen operation for chaining operations
pub struct AndThenOperation<O, F, T, U> {
    inner: O,
    f: F,
    _phantom: PhantomData<(F, T, U)>,
}

impl<O, F, O2, T, U> Operation<U> for AndThenOperation<O, F, T, U>
where
    O: Operation<T>,
    F: FnOnce(T) -> O2 + Send + Sync + Clone + 'static,
    O2: Operation<U> + Send,
    T: Send + 'static,
    U: Send + 'static,
{
    fn execute(&self) -> Pin<Box<dyn Future<Output = OperationResult<U>> + Send + '_>> {
        let inner_future = self.inner.execute();
        let f = self.f.clone();

        Box::pin(async move {
            let inner_result = inner_future.await?;
            let next_operation = f(inner_result);
            next_operation.execute().await
        })
    }

    fn tags(&self) -> Vec<OperationTag> {
        self.inner.tags()
    }
}

/// OrElse operation for error recovery
pub struct OrElseOperation<O, F, T> {
    inner: O,
    f: F,
    _phantom: PhantomData<(F, T)>,
}

impl<O, F, O2, T> Operation<T> for OrElseOperation<O, F, T>
where
    O: Operation<T>,
    F: FnOnce(OperationError) -> O2 + Send + Sync + Clone + 'static,
    O2: Operation<T> + Send,
    T: Send + 'static,
{
    fn execute(&self) -> Pin<Box<dyn Future<Output = OperationResult<T>> + Send + '_>> {
        let inner_future = self.inner.execute();
        let f = self.f.clone();

        Box::pin(async move {
            match inner_future.await {
                Ok(result) => Ok(result),
                Err(error) => {
                    let next_operation = f(error);
                    next_operation.execute().await
                }
            }
        })
    }

    fn tags(&self) -> Vec<OperationTag> {
        self.inner.tags()
    }
}

/// Create a portfolio operation module for portfolio-specific operations
pub mod portfolio {
    use super::*;
    use crate::portfolio::{PortfolioOperation, TokenBalance};

    // Check balances operation
    pub struct CheckBalancesOperation {
        pub context: PortfolioContext,
    }

    impl Operation<Vec<TokenBalance>> for CheckBalancesOperation {
        fn execute(
            &self,
        ) -> Pin<Box<dyn Future<Output = OperationResult<Vec<TokenBalance>>> + Send + '_>> {
            let context = self.context.clone();

            Box::pin(async move {
                // Check balances - this updates the internal state
                context
                    .check_balances()
                    .await
                    .map_err(OperationError::Portfolio)?;

                // Now return the updated balances
                Ok(context.get_balances())
            })
        }

        fn tags(&self) -> Vec<OperationTag> {
            vec![OperationTag::Blockchain, OperationTag::Network]
        }
    }

    // Calculate quotes operation
    pub struct CalculateQuotesOperation {
        pub context: PortfolioContext,
    }

    impl Operation<()> for CalculateQuotesOperation {
        fn execute(&self) -> Pin<Box<dyn Future<Output = OperationResult<()>> + Send + '_>> {
            let context = self.context.clone();

            Box::pin(async move {
                // Simply call the calculate_quotes method which handles the await properly
                context
                    .calculate_quotes()
                    .await
                    .map_err(OperationError::Portfolio)
            })
        }

        fn tags(&self) -> Vec<OperationTag> {
            vec![OperationTag::Blockchain, OperationTag::Network]
        }
    }

    // Execute swaps operation
    pub struct ExecuteSwapsOperation {
        pub context: PortfolioContext,
    }

    impl Operation<()> for ExecuteSwapsOperation {
        fn execute(&self) -> Pin<Box<dyn Future<Output = OperationResult<()>> + Send + '_>> {
            let context = self.context.clone();

            Box::pin(async move {
                // Simply call the execute_swaps method which handles the await properly
                context
                    .execute_swaps()
                    .await
                    .map_err(OperationError::Portfolio)
            })
        }

        fn tags(&self) -> Vec<OperationTag> {
            vec![OperationTag::Blockchain, OperationTag::Network]
        }
    }

    // Start operation operation
    pub struct StartOperationOperation {
        pub context: PortfolioContext,
        pub operation: PortfolioOperation,
    }

    impl Operation<()> for StartOperationOperation {
        fn execute(&self) -> Pin<Box<dyn Future<Output = OperationResult<()>> + Send + '_>> {
            let context = self.context.clone();
            let operation = self.operation.clone();

            Box::pin(async move {
                // Simply call start_operation which handles awaits properly
                context
                    .start_operation(operation)
                    .await
                    .map_err(OperationError::Portfolio)
            })
        }

        fn tags(&self) -> Vec<OperationTag> {
            vec![OperationTag::Blockchain, OperationTag::Network]
        }
    }
}

/// DeFi operations module for DeFi protocol interactions
/// Contains:
///  - `aave_health`: Module for Aave health check operations using state machine approach
pub mod defi;

/// Token operations module for token-specific operations
pub mod token;
