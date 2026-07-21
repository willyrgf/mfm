# mfm-adapters-evm

Runtime and replay bindings for exactly four reusable EVM state kinds:
`CollectEvmBalancesState`, `RecordEvmBalanceFactsState`, `SubmitEvmTransactionState`, and
`ValidateEvmContractState`.

`EvmReadRunnerCapabilities` is the single source-bound read assembly used by balance collection and
exact-anchor validation. Both families await its one asynchronous route validator before admission;
failed validation cannot bind a session or issue an RPC. The balance executor resolves one anchor,
performs bounded ordered native and ERC-20 reads, and retains complete reducer evidence. The record
runner atomically publishes the complete `evm.balance_snapshot` batch with its checked receipt.

Registration is deliberately split: `register_evm_balance_runners`,
`register_evm_validation_runner`, and `register_evm_transaction_runner` let each consumer install
only the state family it owns. The production app selects balance registration only; validation and
transaction registration remain explicit library/test foundations.

`verify_evm_balance_collection_replay` recomputes the read output, fact batch, receipt, and recorded
fact evidence without runtime config or a live route. Transaction and validation replay remain in
this same adapter package. Operation topology is intentionally absent.
