#!/usr/bin/env python3
"""Create inert local targets for the dependency-only Cargo build.

The dependency pilot intentionally filters workspace Rust sources out of its
Nix source input. Cargo still needs every workspace manifest to describe a
valid target graph, so this script creates empty local library and explicitly
declared target files in the disposable build tree. The resulting derivation
measures third-party compilation without pretending to build final MFM
artifacts.
"""

from __future__ import annotations

import pathlib
import sys
import tomllib


def target_paths(manifest: pathlib.Path, package: dict) -> set[pathlib.Path]:
    """Return the target paths Cargo requires while parsing this package."""

    paths: set[pathlib.Path] = {manifest.parent / "src/lib.rs"}

    library = package.get("lib")
    if library is not None and "path" in library:
        paths.add(manifest.parent / library["path"])

    for section, default_directory in (
        ("bin", "src/bin"),
        ("test", "tests"),
        ("example", "examples"),
        ("bench", "benches"),
    ):
        for target in package.get(section, []):
            default_name = target["name"]
            default_path = f"{default_directory}/{default_name}.rs"
            paths.add(manifest.parent / target.get("path", default_path))

    return paths


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: dependency-artifact-stubs.py SOURCE_ROOT")

    root = pathlib.Path(sys.argv[1]).resolve()
    for manifest in root.rglob("Cargo.toml"):
        document = tomllib.loads(manifest.read_text(encoding="utf-8"))
        package = document.get("package")
        if package is None:
            continue

        for target in target_paths(manifest, document):
            target.parent.mkdir(parents=True, exist_ok=True)
            target.touch(exist_ok=True)


if __name__ == "__main__":
    main()
