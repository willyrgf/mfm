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

### get a yield farms info

```bash
cargo run -- yield-farm info
```

```bash
cargo run -- yield-farm info --quoted-asset busd
```

to filter by yield farm name

```bash
cargo run -- yield-farm info -y auto-cake
```

---

## harvesting yield farms

### Current supported farms

| Farm                      | operation           | Contract                                   |
|---------------------------|---------------------|--------------------------------------------|
| Position Farm BNB/POSI    | posi_farm_bnb_posi  | 0x0c54b0b7d61de871db47c3ad3f69feb0f2c8db0b |
| Position Farm BUSD/POSI   | posi_farm_busd_posi | 0x0c54b0b7d61de871db47c3ad3f69feb0f2c8db0b |
| PancakeSwap AutoCake Pool | cake_auto_pool      | 0xa80240eb5d7e05d3f250cf000eec0891d00b51cc |
| Paçoca Auto Paçoca Pool | pacoca_auto_pool      | 0x16205528a8f7510f4421009a7654835b541bb1b9 |
| QI DAO QI-MATIC Pool  | qi_dao_staking_pool_qi_wmatic | 0x574fe4e8120c4da1741b5fd45584de7a5b521f0f |

use run to run over all farms configured

```bash
cargo run -- yield-farm run
```

to filter by yield farm name

```bash
cargo run -- yield-farm run -y auto-cake
```

to skip the min rewards required configured and force the harvest

```bash
cargo run -- yield-farm run --force-harvest true
```

---

## todo
- [ ] refactor of the config mods to be first class module (like asset)
- [ ] refactor all the U256 calc to use numbigint in testable functions
- [ ] refactor wallet and withdraw-wallet to be wallet with private and public address supporting encrypted files with the keys
- [ ] doc new rebalancer diff parking
- [x] compile /res resources assets/networks/exchanges configs and fallback to local configs
- [x] add a rebalance that use only the diff between the assets to rebalance the portfolio
- [x] add withdraw with wallets whilelist
- [x] add harverst for yield farms
- [x] implement rebalancer threshold
- [x] implement a fallback to a bep20 abi
- [x] implement a checker for token limit transfer (max-tx-amount)
