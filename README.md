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

### swap tokens for tokens supporting fees on transfer
```
 cargo run -- swaptt -a 0.0006 -e pancake_swap_v2 -w test-wallet -i wbnb -o busd
```