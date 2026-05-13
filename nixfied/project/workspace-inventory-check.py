#!/usr/bin/env python3
"""Validate the Cargo workspace inventory before metadata consumers run."""

from __future__ import annotations

import argparse
import collections
import pathlib
import subprocess
import sys
import tomllib

PACKAGE_MANIFEST_ROOTS = ("bin", "crates", "tests")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Validate Cargo workspace members for source-of-truth hygiene.",
    )
    location = parser.add_mutually_exclusive_group(required=True)
    location.add_argument(
        "--root",
        type=pathlib.Path,
        help="Workspace root containing Cargo.toml",
    )
    location.add_argument(
        "--manifest",
        type=pathlib.Path,
        help="Path to the workspace Cargo.toml",
    )
    parser.add_argument(
        "--metadata",
        action="store_true",
        help="Run cargo metadata after manifest validation succeeds.",
    )
    return parser.parse_args()


def manifest_path(args: argparse.Namespace) -> pathlib.Path:
    if args.manifest is not None:
        return args.manifest
    return args.root / "Cargo.toml"


def load_toml(path: pathlib.Path) -> dict:
    try:
        raw = path.read_text(encoding="utf-8")
    except OSError as error:
        raise ValueError(f"failed to read Cargo manifest path={path}: {error}") from error

    try:
        return tomllib.loads(raw)
    except tomllib.TOMLDecodeError as error:
        raise ValueError(f"failed to parse Cargo manifest path={path}: {error}") from error


def normalize_member(member: str) -> str:
    path = pathlib.PurePosixPath(member)
    if path.is_absolute() or ".." in path.parts:
        raise ValueError(f"Cargo workspace member path must stay inside workspace: {member}")

    normalized = path.as_posix()
    if normalized in {"", "."}:
        raise ValueError(f"Cargo workspace member path must not be empty: {member}")

    return normalized


def load_members(path: pathlib.Path) -> list[str]:
    document = load_toml(path)

    workspace = document.get("workspace")
    if not isinstance(workspace, dict):
        raise ValueError(f"Cargo manifest has no [workspace] table path={path}")

    members = workspace.get("members")
    if not isinstance(members, list):
        raise ValueError(f"Cargo workspace members must be an array path={path}")

    invalid_members = [member for member in members if not isinstance(member, str)]
    if invalid_members:
        raise ValueError(f"Cargo workspace members must be strings path={path}")

    return [normalize_member(member) for member in members]


def discover_package_members(root: pathlib.Path) -> list[str]:
    discovered: set[str] = set()
    for root_name in PACKAGE_MANIFEST_ROOTS:
        search_root = root / root_name
        if not search_root.exists():
            continue

        for manifest in search_root.glob("**/Cargo.toml"):
            if not manifest.is_file():
                continue

            document = load_toml(manifest)
            if not isinstance(document.get("package"), dict):
                raise ValueError(
                    f"discovered Cargo package manifest has no [package] table path={manifest}"
                )

            discovered.add(manifest.parent.relative_to(root).as_posix())

    return sorted(discovered)


def duplicate_members(members: list[str]) -> list[str]:
    counts = collections.Counter(members)
    return sorted(member for member, count in counts.items() if count > 1)


def validate_members(members: list[str], discovered_members: list[str]) -> None:
    duplicates = duplicate_members(members)
    if duplicates:
        joined = ", ".join(duplicates)
        raise ValueError(f"duplicate Cargo workspace members: {joined}")

    member_set = set(members)
    discovered_set = set(discovered_members)
    missing_members = sorted(discovered_set - member_set)
    if missing_members:
        joined = ", ".join(missing_members)
        raise ValueError(f"Cargo workspace missing discovered members: {joined}")

    dead_members = sorted(member_set - discovered_set)
    if dead_members:
        joined = ", ".join(dead_members)
        raise ValueError(f"Cargo workspace dead members: {joined}")


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
        root = args.root if args.root is not None else path.parent
        discovered_members = discover_package_members(root)
        validate_members(members, discovered_members)
        if args.metadata:
            run_metadata(root)
    except ValueError as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 1

    print(f"OK: Cargo workspace inventory valid path={path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
