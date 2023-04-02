# MFM

MFM (Multiverse Finance Machine) is a CLI to managing portfolio of cryptoassets directly in Blockchain using DEXs.

> **WARNING**: This project is in an alpha stage. It has not been audited and may contain bugs and security flaws. This implementation is NOT ready for production use.

## Big features coming
- [ ] Support LP's as a asset type and handle with them in the portfolio
- [ ] Support Yield Farms and Harvest rewards
- [ ] Add machine command within a module using state-machine logic to run sequencially and conditionally multiples commands as workflows
- [ ] Add automatic hedge using levarage derivatives (futures/options)
- [ ] Trading strategies with TA (in [mfm_server](https://github.com/willyrgf/mfm_server)?) to adjust dynamic portfolio positions

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

## Local test environment

### run GETH server
```sh
 docker build -t geth_local tests/blockchains/gethnet
 docker run --name geth_local -d  -p 8545:8545 -p 8546:8546 geth_local
```

### stop & drop GETH server
```sh
docker stop geth_local && docker rm geth_local
```

### extract base wallet private key
```sh
$ ethkey inspect --private --passwordfile tests/blockchains/gethnet/password.txt tests/blockchains/gethnet/data/keystore/UTC--2023-03-28T01-13-34.803419000Z--4e22e05c29a7165aeee0d813be03af17f129a2d1
Address:        0x4E22e05C29A7165Aeee0D813bE03Af17F129A2d1
Public key:     04bf840d4f25bda6e5cf46dfe8177f33f8ea5ab5e90d17992272c6b3a931976f3a6328d513e943becbc4f9e46d89bce4f9c9654698252d7b09469015ea2c36862d
Private key:    afcaf34d4647a3e50b39029fb34aa94b59ae75606f57d78e4bcb286948ed4816
```



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
 cargo run -- quote --network bsc --exchange pancake_swap_v2  -i wbnb -o busd -a 1.0
```

---

### swap tokens for tokens supporting fees on transfer

```bash
 cargo run -- swap -w test-wallet -n bsc -e pancake_swap_v2 -i wbnb -o busd -a 0.0006
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
cargo run -- withdraw --wallet test-wallet --network bsc --withdraw-wallet test-wallet2 -v 0.008 -a wbnb
```
