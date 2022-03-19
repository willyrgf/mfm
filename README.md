# MFM

# cmd examples

### wrap token (ex: in bsc bnb -> wbnb, in eth eth -> weth, in polygon matic -> wmatic )
```
 cargo run -- wrap --network bsc --wallet test-wallet --amount 0.005
```

### check allowance data for an asset
```
 cargo run -- allowance -e pancake_swap_v2 -w test-wallet -a wbnb
```
### approve spender for an asset
```
 cargo run -- approve_spender -e pancake_swap_v2 -w test-wallet -a wbnb -v 10
```
### swap tokens for tokens supporting fees on transfer
```
 cargo run -- swap -a 0.0006 -e pancake_swap_v2 -w test-wallet -i wbnb -o busd
```
### get balances of assets from wallet
```
 cargo run -- balances -w test-wallet
```
### run rebalancer
```
 cargo run -- rebalancer -n test-rebalancer
```

### run withdraw
```
cargo watch -x 'run -- withdraw --wallet test-wallet -t test-wallet2 -v 0.008 -a wbnb -n bsc'
```

# todo

- [x] add withdraw with wallets whilelist
- [x] add harverst for yield farms
- [ ] implement rebalancer threshold
- [ ] implement a checker for token limit transfer (max-tx-amount)
- [ ] implement a fallback to a bep20 abi
- [ ] implement 