# MFM

## cmd examples

### wrap token (ex: in bsc bnb -> wbnb, in eth eth -> weth, in polygon matic -> wmatic )

```bash
cargo run -- wrap --network bsc --wallet test-wallet --amount 0.005
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

- [x] add withdraw with wallets whilelist
- [x] add harverst for yield farms
- [x] implement rebalancer threshold
- [ ] implement a checker for token limit transfer (max-tx-amount)
- [ ] implement a fallback to a bep20 abi
