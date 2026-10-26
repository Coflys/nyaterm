#!/usr/bin/env python3
"""Update the Cargo workspace release version without resolving dependencies."""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path

import package_native


ROOT_DIR = Path(__file__).resolve().parents[2]
TABLE_HEADER = re.compile(r"(?m)^[ \t]*\[(?P<table>[^\r\n]+)\][ \t]*(?:#[^\r\n]*)?\r?$")
PACKAGE_HEADER = re.compile(r"(?m)^\[\[package\]\][ \t]*\r?$")
VERSION_FILES = ("Cargo.toml", "Cargo.lock")


@dataclass(frozen=True)
class Change:
    path: Path
    before: bytes
    after: bytes


def replace_version(text: str, version: str) -> str:
    # Edit only the value so comments, quoting, and line endings survive.
    pattern = r"(?m)^([ \t]*version[ \t]*=[ \t]*)([\"'])([^\"'\r\n]+)(\2)"
    matches = list(re.finditer(pattern, text))
    if len(matches) != 1:
        raise ValueError("expected exactly one version assignment in the table")
    match = matches[0]
    return text[:match.start(3)] + version + text[match.end(3):]


def inherited_packages(root: Path, workspace: dict) -> set[str]:
    excluded = {
        path.resolve()
        for pattern in workspace.get("exclude", [])
        for path in root.glob(pattern)
    }
    names = set()
    for pattern in workspace["members"]:
        members = sorted(root.glob(pattern))
        if not members:
            raise ValueError(f"workspace member not found: {pattern}")
        for member in members:
            member = member.resolve()
            if not member.is_relative_to(root):
                raise ValueError(f"workspace member outside repository: {pattern}")
            if any(member.is_relative_to(path) for path in excluded):
                continue
            manifest = tomllib.loads((member / "Cargo.toml").read_text(encoding="utf-8"))
            package = manifest.get("package", {})
            if package.get("version") == {"workspace": True}:
                names.add(package["name"])
    if not names:
        raise ValueError("no packages inherit the workspace version")
    return names


def update_lock(text: str, names: set[str], old: str, new: str) -> str:
    headers = list(PACKAGE_HEADER.finditer(text))
    result = [text[:headers[0].start()] if headers else text]
    found = set()
    for index, header in enumerate(headers):
        end = headers[index + 1].start() if index + 1 < len(headers) else len(text)
        block = text[header.start():end]
        package = tomllib.loads(block)["package"][0]
        name = package["name"]
        if name in names and "source" not in package:
            if name in found:
                raise ValueError(f"duplicate local package in Cargo.lock: {name}")
            if package["version"] not in {old, new}:
                raise ValueError(f"unexpected Cargo.lock version for {name}: {package['version']}")
            found.add(name)
            block = replace_version(block, new)
        result.append(block)
    if missing := names - found:
        raise ValueError(f"workspace packages missing from Cargo.lock: {', '.join(sorted(missing))}")
    updated = "".join(result)
    # Cargo qualifies dependency names with versions when names are ambiguous.
    # Source-qualified references identify external packages and stay untouched.
    for name in sorted(names):
        updated = updated.replace(f'"{name} {old}"', f'"{name} {new}"')
    tomllib.loads(updated)
    return updated


def plan_changes(root: Path, raw_version: str) -> tuple[str, str, set[str], list[Change]]:
    new = package_native.validate_version(raw_version)
    root = root.resolve()
    manifest_path = root / "Cargo.toml"
    manifest_bytes = manifest_path.read_bytes()
    manifest_text = manifest_bytes.decode("utf-8")
    workspace = tomllib.loads(manifest_text)["workspace"]
    old = workspace["package"]["version"]
    names = inherited_packages(root, workspace)
    headers = list(TABLE_HEADER.finditer(manifest_text))
    for index, header in enumerate(headers):
        if header.group("table") == "workspace.package":
            end = headers[index + 1].start() if index + 1 < len(headers) else len(manifest_text)
            table = replace_version(manifest_text[header.end():end], new)
            updated_manifest = manifest_text[:header.end()] + table + manifest_text[end:]
            break
    else:
        raise ValueError("[workspace.package] table not found")
    if tomllib.loads(updated_manifest)["workspace"]["package"]["version"] != new:
        raise ValueError("failed to update workspace version")

    lock_path = root / "Cargo.lock"
    lock_bytes = lock_path.read_bytes()
    updated_lock = update_lock(lock_bytes.decode("utf-8"), names, old, new)
    changes = [
        Change(path, before, after.encode("utf-8"))
        for path, before, after in (
            (manifest_path, manifest_bytes, updated_manifest),
            (lock_path, lock_bytes, updated_lock),
        )
        if before != after.encode("utf-8")
    ]
    return old, new, names, changes


def git(root: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", *args], cwd=root, capture_output=True, text=True, encoding="utf-8", errors="replace"
    )
    if result.returncode:
        raise ValueError(result.stderr.strip() or result.stdout.strip() or f"git {args[0]} failed")
    return result.stdout.strip()


def check_commit_ready(root: Path) -> None:
    if Path(git(root, "rev-parse", "--show-toplevel")).resolve() != root.resolve():
        raise ValueError("version files must be at the Git repository root")
    if not git(root, "branch", "--show-current"):
        raise ValueError("--commit requires a branch; HEAD is detached")
    for operation in ("MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD", "rebase-merge", "rebase-apply"):
        path = Path(git(root, "rev-parse", "--git-path", operation))
        if (root / path).exists():
            raise ValueError(f"finish the active Git operation before --commit: {operation}")
    if git(root, "ls-files", "--unmerged"):
        raise ValueError("resolve Git conflicts before --commit")
    git(root, "ls-files", "--error-unmatch", "--", *VERSION_FILES)
    if git(root, "status", "--porcelain=v1", "--", *VERSION_FILES):
        raise ValueError("--commit requires clean Cargo.toml and Cargo.lock; commit existing edits first")
    # Ask Git to validate the configured identity without changing configuration.
    git(root, "var", "GIT_AUTHOR_IDENT")
    git(root, "var", "GIT_COMMITTER_IDENT")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version", help="release SemVer, optionally prefixed with v")
    parser.add_argument("--dry-run", action="store_true", help="preview changes without writing files")
    parser.add_argument("--commit", action="store_true", help="commit only the version files after updating")
    args = parser.parse_args(argv)
    try:
        old, new, names, changes = plan_changes(ROOT_DIR, args.version)
        message = f"chore(release): bump version to {new}"
        if args.commit and changes:
            check_commit_ready(ROOT_DIR)
        print(f"Workspace version: {old} -> {new} ({len(names)} inherited packages)")
        for change in changes:
            print(f"{'Would update' if args.dry_run else 'Update'} {change.path.relative_to(ROOT_DIR)}")
        if not args.dry_run:
            for change in changes:
                if change.path.read_bytes() != change.before:
                    raise ValueError(f"file changed during version planning: {change.path.name}")
            for change in changes:
                change.path.write_bytes(change.after)
        if args.commit and changes:
            if args.dry_run:
                print(f"Would commit: {message}")
            else:
                try:
                    # --only preserves unrelated staged changes in the user's index.
                    # Files retain their original line endings. With autocrlf off,
                    # diff --check otherwise treats the CR in CRLF as whitespace.
                    whitespace = git(
                        ROOT_DIR, "config", "--default", "trailing-space,space-before-tab",
                        "--get", "core.whitespace",
                    )
                    git(
                        ROOT_DIR, "-c", f"core.whitespace={whitespace},cr-at-eol",
                        "diff", "--check", "--", *VERSION_FILES,
                    )
                    git(ROOT_DIR, "commit", "--only", "-m", message, "--", *VERSION_FILES)
                except (OSError, ValueError) as error:
                    raise ValueError(f"commit failed; version edits remain available for review: {error}") from error
                print(f"Committed {git(ROOT_DIR, 'rev-parse', '--short', 'HEAD')}: {message}")
        if not changes:
            print("Versions are already up to date.")
    except (OSError, ValueError, KeyError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
