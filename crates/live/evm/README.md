# mfm-evm-live

Runtime and replay bindings for exactly three reusable EVM state kinds:
`CollectEvmBalancesState`, `SubmitEvmTransactionState`, and `ValidateEvmContractState`.

`EvmReadRunnerCapabilities` is the single source-bound read assembly used by balance collection and
exact-anchor validation. Both families resolve a checked session from one supplied session set
before admission. The balance executor resolves one anchor, performs bounded ordered native and
ERC-20 reads, and retains complete reducer evidence. The read reducer returns the complete
`evm.balance_snapshot` batch with its checked receipt for atomic runtime settlement.

Registration is deliberately split: `register_evm_balance_runners`,
`register_evm_validation_runner`, and `register_evm_transaction_runner` let each consumer install
only the state family it owns. The production app selects balance registration only; validation and
transaction registration remain explicit library/test foundations.

`verify_evm_balance_collection_replay` recomputes the read output, fact batch, receipt, and recorded
fact evidence without runtime config or a live route. Transaction and validation replay remain in
the private adapter module. The public transport is independently reusable without runtime
registration. Operation topology is intentionally absent.
