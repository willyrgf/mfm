#!/usr/bin/env bash
set -euo pipefail

DISALLOWED_PATTERN='std::fs::|std::env::|rpassword::|io::stdin\(|\bread_password\('

violations=0

while IFS= read -r file; do
  # Only scan impl State blocks in non-test sections.
  if awk '
    BEGIN { in_impl = 0; depth = 0; in_test = 0 }
    /^#\[cfg\(test\)\]/ { in_test = 1 }
    in_test { next }
    {
      if (!in_impl && $0 ~ /impl[[:space:]]+State[[:space:]]+for[[:space:]]+/) {
        in_impl = 1
        depth = 0
      }
      if (in_impl) {
        line = $0
        open_count = gsub(/\{/, "{", line)
        close_count = gsub(/\}/, "}", line)
        depth += open_count - close_count
        print NR ":" $0
        if (depth <= 0) {
          in_impl = 0
          depth = 0
        }
      }
    }
  ' "$file" | rg -n "${DISALLOWED_PATTERN}" >/tmp/mfm-ambient-io-check.txt; then
    echo "ambient-io violation in state handler file: $file" >&2
    cat /tmp/mfm-ambient-io-check.txt >&2
    violations=1
  fi
done < <(rg -l 'impl[[:space:]]+State[[:space:]]+for[[:space:]]+' crates/ops --glob '**/*.rs')

if [[ $violations -ne 0 ]]; then
  echo "error: found ambient IO usage in state handlers; route via local IO transport abstractions" >&2
  exit 1
fi

exit 0
