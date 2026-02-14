#!/usr/bin/env bash
set -euo pipefail

PATTERN='println!|eprintln!|dbg!'

if rg -n "${PATTERN}" bin crates tests --glob '**/*.rs' \
  --glob '!bin/cli/src/presentation/output.rs' \
  --glob '!bin/cli/src/support/input.rs' \
  --glob '!bin/cli/src/main.rs' \
  --glob '!crates/core/src/keystore/mod.rs'; then
  echo "error: found ad-hoc print/debug macros outside approved output/prompt locations" >&2
  exit 1
fi

exit 0
