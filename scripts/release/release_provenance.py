#!/usr/bin/env python3
"""Bind package artifacts to the actual checkout, version and producing run."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

import generate_release_metadata
import package_native


def identity(source_sha: str, version: str, run_id: str) -> dict[str, object]:
    if not re.fullmatch(r"[0-9a-f]{40}", source_sha):
        raise ValueError("source SHA must be a full commit SHA")
    if not re.fullmatch(r"[1-9][0-9]*", run_id):
        raise ValueError("run ID must be a positive integer")
    return {
        "schema_version": 1,
        "source_sha": source_sha,
        "version": package_native.validate_version(version),
        "run_id": run_id,
    }


def record(dist: Path, target: str, source_sha: str, version: str, run_id: str) -> dict:
    if target not in package_native.TARGETS:
        raise ValueError(f"unknown target: {target}")
    result = identity(source_sha, version, run_id)
    result["target"] = target
    result["artifacts"] = {
        name: generate_release_metadata.sha256(dist / name)
        for name in sorted(package_native.artifact_names(target, version))
    }
    return result


def verify(dist: Path, records: Path, source_sha: str, version: str, run_id: str) -> None:
    expected_records = {f"{target}.json" for target in package_native.TARGETS}
    if {p.name for p in records.iterdir()} != expected_records:
        raise ValueError("incomplete or unexpected provenance records; rebuild all packages")
    for target in package_native.TARGETS:
        actual = json.loads((records / f"{target}.json").read_text(encoding="utf-8"))
        if actual != record(dist, target, source_sha, version, run_id):
            raise ValueError(f"package provenance mismatch for {target}; rebuild packages")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("record", "verify"))
    parser.add_argument("--dist", type=Path, required=True)
    parser.add_argument("--records", type=Path, required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--target")
    args = parser.parse_args()
    if args.operation == "record":
        result = record(args.dist, args.target, args.source_sha, args.version, args.run_id)
        args.records.mkdir(parents=True, exist_ok=True)
        (args.records / f"{args.target}.json").write_text(
            json.dumps(result, indent=2) + "\n", encoding="utf-8"
        )
    else:
        verify(args.dist, args.records, args.source_sha, args.version, args.run_id)


if __name__ == "__main__":
    main()
