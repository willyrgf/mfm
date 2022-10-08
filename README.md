# MFM

## Fast local install & update using releases

### *nix (?)
```sh
# may need adjust for some systems
LATEST_APP_URL="$( \
	curl -s https://api.github.com/repos/willyrgf/mfm/releases/latest | 
	grep 'browser_download_url' | 
	grep "$(uname | tr '[[:upper:]]' '[[:lower:]]')" | 
	awk -F '"' '!/.sha256sum/ {print $4}' \
)"
curl -s -L $LATEST_APP_URL -O
unzip -qo ${LATEST_APP_URL##*/}

```

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
 cargo run -- approve -e pancake_swap_v2 -w test-wallet -a wbnb -v 10
```


#### To approve all assets
```bash
cargo run -- allowance --network polygon --wallet test-wallet | 
	grep ^\| | 
	grep -v Exchange | 
	awk -F '|' '{print $2 $3}' | 
	xargs -n 2 bash -c 'cargo run -- approve --exchange $0 -w test-wallet --asset $1 --amount 10000000'
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
