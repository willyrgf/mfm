#!/usr/bin/env python3
"""Validate the Cargo workspace inventory before metadata consumers run."""

from __future__ import annotations

import argparse
import collections
import pathlib
import subprocess
import sys
import tomllib


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Validate Cargo workspace members for source-of-truth hygiene.",
    )
    location = parser.add_mutually_exclusive_group(required=True)
    location.add_argument("--root", type=pathlib.Path, help="Workspace root containing Cargo.toml")
    location.add_argument("--manifest", type=pathlib.Path, help="Path to the workspace Cargo.toml")
    parser.add_argument(
        "--metadata",
        action="store_true",
        help="Run cargo metadata after manifest validation succeeds.",
    )
    parser.add_argument(
        "--expect-member-once",
        action="append",
        default=[],
        metavar="PATH",
        help="Assert a workspace member appears exactly once.",
    )
    return parser.parse_args()


def manifest_path(args: argparse.Namespace) -> pathlib.Path:
    if args.manifest is not None:
        return args.manifest
    return args.root / "Cargo.toml"


def load_members(path: pathlib.Path) -> list[str]:
    try:
        raw = path.read_text(encoding="utf-8")
    except OSError as error:
        raise ValueError(f"failed to read Cargo manifest path={path}: {error}") from error

    try:
        document = tomllib.loads(raw)
    except tomllib.TOMLDecodeError as error:
        raise ValueError(f"failed to parse Cargo manifest path={path}: {error}") from error

    workspace = document.get("workspace")
    if not isinstance(workspace, dict):
        raise ValueError(f"Cargo manifest has no [workspace] table path={path}")

    members = workspace.get("members")
    if not isinstance(members, list):
        raise ValueError(f"Cargo workspace members must be an array path={path}")

    invalid_members = [member for member in members if not isinstance(member, str)]
    if invalid_members:
        raise ValueError(f"Cargo workspace members must be strings path={path}")

    return members


def duplicate_members(members: list[str]) -> list[str]:
    counts = collections.Counter(members)
    return sorted(member for member, count in counts.items() if count > 1)


def validate_members(members: list[str], expected_once: list[str]) -> None:
    duplicates = duplicate_members(members)
    if duplicates:
        joined = ", ".join(duplicates)
        raise ValueError(f"duplicate Cargo workspace members: {joined}")

    counts = collections.Counter(members)
    missing_or_repeated = [
        member for member in expected_once if counts.get(member, 0) != 1
    ]
    if missing_or_repeated:
        details = ", ".join(
            f"{member} count={counts.get(member, 0)}" for member in missing_or_repeated
        )
        raise ValueError(f"Cargo workspace member cardinality mismatch: {details}")


def run_metadata(root: pathlib.Path) -> None:
    result = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=root,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        message = result.stderr.strip()
        if message:
            raise ValueError(f"cargo metadata failed: {message}")
        raise ValueError(f"cargo metadata failed with status {result.returncode}")


def main() -> int:
    args = parse_args()
    path = manifest_path(args)
    try:
        members = load_members(path)
        validate_members(members, args.expect_member_once)
        if args.metadata:
            root = args.root if args.root is not None else path.parent
            run_metadata(root)
    except ValueError as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1

    print(f"OK: Cargo workspace inventory valid path={path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
