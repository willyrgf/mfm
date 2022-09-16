# MFM

## res/ directory
This directory will carry all the abi files used as default in this project.
All these abi files in this directory will be add in the binary in compilation time (`build.rs`).

To access this files we've a `shared::resources` module that will always condering your currently
filesystem `res/` directory and the default `static RES` compiled in the binary (`build.rs`), 
following this order  of priority respectively.

<!-- TODO: add install doc and res folder -->

## cmd examples

### wrap token (ex: in bsc bnb -> wbnb, in eth eth -> weth, in polygon matic -> wmatic )

```bash
cargo run -- wrap --network bsc --wallet test-wallet --amount 0.005
```

---

### unwrap token (ex: in bsc wbnb -> bnb, in eth weth -> eth, in polygon wmatic -> matic )

```bash
cargo run -- unwrap --network bsc --wallet test-wallet --amount 0.005
```

---

### check allowance data for an asset

```bash
 cargo run -- allowance -e pancake_swap_v2 -w test-wallet -a wbnb
```

---

### approve spender for an asset

```bash
 cargo run -- approve_spender -e pancake_swap_v2 -w test-wallet -a wbnb -v 10
```

---

### get a quote

```bash
 cargo run -- quote -a 1 -e pancake_swap_v2 -i wbnb -o busd
```

---

### swap tokens for tokens supporting fees on transfer

```bash
 cargo run -- swap -a 0.0006 -e pancake_swap_v2 -w test-wallet -i wbnb -o busd
```

---

### get balances of assets from wallet

```bash
 cargo run -- balances -w test-wallet
```

---

### run rebalancer

```bash
 cargo run -- rebalancer -n test-rebalancer
```

---

### run withdraw

```bash
cargo run -- withdraw --wallet test-wallet -t test-wallet2 -v 0.008 -a wbnb -n bsc
```

---

## TODO
- [ ] refactor all the U256 calc to use numbigint in testable functions
- [ ] refactor of the config mods to be first class module (like asset)
- [ ] refactor the encrypted wallet and document it (may like that https://github.com/tari-project/tari/blob/c86727969ef3fffc124ab706d44c8845addbf415/applications/tari_console_wallet/src/cli.rs#L54)
- [ ] use `get_better_exchange` in swap mod when exchange is not provided
- [ ] start to release the app and document the process to update and run locally
- [ ] add a shutdown control to signals (may like that https://github.com/tari-project/tari/blob/77bb10d42e8c004406d0ddd69b65575f0e111cd1/applications/tari_console_wallet/src/main.rs#L139)
- [ ] add a transaction command to check status of a transaction id in the blockchain (feat/transaction)
- [ ] add a yield farm module to interact with yield farm contracts (feat/yield-farm)
- [ ] doc commands
- [x] check the better exchange based on liquidity `exchange_to_use()`
- [x] refactor wallet and withdraw-wallet to be wallet with private and public address supporting encrypted files with the keys
- [x] compile /res resources assets/networks/exchanges configs and fallback to local configs
- [x] add a rebalance that use only the diff between the assets to rebalance the portfolio
- [x] add withdraw with wallets whilelist
- [x] add harverst for yield farms
- [x] implement rebalancer threshold
- [x] implement a fallback to a bep20 abi
- [x] implement a checker for token limit transfer (max-tx-amount)
