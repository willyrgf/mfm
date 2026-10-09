#!/usr/bin/env python3
"""Run the frozen specimen's managed task and preserve its temporary public dumps.

Archival plumbing only: the pinned task uses TMPDIR=${stateDir}, whose cleanup
does not follow symlinks. Link only the dump directory to a fresh owned location;
do not change candidate source, native resources, authority or observer code.
The full dumps include generated fixture bytecode and must remain temporary.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--worktree", type=Path, required=True)
parser.add_argument("--output", type=Path, required=True)
args = parser.parse_args()
output = args.output.resolve()
output.mkdir(mode=0o700, parents=True, exist_ok=False)
dumps = output / "dumps"
dumps.mkdir(mode=0o700)
state_base = output / "state"
state = state_base / "data/mfm/dev/0"
environment = dict(os.environ, NIXFIED_STATE_DIR=str(state_base))
log_path = output / "effect-export.log"
with log_path.open("wb") as log:
    process = subprocess.Popen(
        ["nix", "run", ".#effect-e2e"],
        cwd=args.worktree.resolve(), env=environment,
        stdout=log, stderr=subprocess.STDOUT,
    )
    linked = False
    while process.poll() is None:
        if not linked and state.is_dir():
            (state / "mfm-effect-specimen-dumps").symlink_to(dumps, target_is_directory=True)
            linked = True
        time.sleep(0.05)
    status = process.wait()
if status:
    raise SystemExit(status)
assert linked, "managed task never exposed its expected state root"
traces = re.findall(
    r"H4 ([\w-]+) bytes=(\d+) digest=content:sha256-v1:([a-f0-9]+)",
    log_path.read_text(),
)
assert traces, "managed task produced no specimen traces"
for name, length, digest in traces:
    raw = (dumps / (name + ".json")).read_bytes()
    json.loads(raw)
    assert len(raw) == int(length), name
    assert hashlib.sha256(raw).hexdigest() == digest, name
print(f"effect-e2e exit {status}; preserved and verified {len(traces)} trace files")
