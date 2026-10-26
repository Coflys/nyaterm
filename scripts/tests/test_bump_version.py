from __future__ import annotations

import contextlib
import io
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path
from unittest import mock


RELEASE_SCRIPTS = Path(__file__).resolve().parents[1] / "release"
sys.path.insert(0, str(RELEASE_SCRIPTS))

import bump_version  # noqa: E402


OLD = "2.0.0-preview.4"
NEW = "2.0.0-preview.5"


class BumpVersionTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        manifest = (
            '[workspace]\nmembers = ["crates/*"]\nexclude = ["crates/excluded"]\n\n'
            f"[workspace.package] # release identity\nversion = '{OLD}' # keep comment\n"
            'edition = "2024"\n\n[workspace.dependencies]\nexternal = "1.0.0"\n'
        )
        (self.root / "Cargo.toml").write_bytes(manifest.replace("\n", "\r\n").encode())
        for name, version in (
            ("app", "version.workspace = true"),
            ("core", "version = { workspace = true }"),
            ("sdk", 'version = "1.0.0"'),
            ("excluded", "version.workspace = true"),
        ):
            directory = self.root / "crates" / name
            directory.mkdir(parents=True)
            (directory / "Cargo.toml").write_text(
                f'[package]\nname = "{name}"\n{version}\n', encoding="utf-8"
            )
        lock = (
            '# generated lockfile\nversion = 4\n\n'
            f'[[package]]\nname = "app"\nversion = "{OLD}"\n'
            f'dependencies = ["core {OLD}", "core {OLD} (registry+https://example.org)", "sdk"]\n\n'
            f'[[package]]\nname = "core"\nversion = "{OLD}"\n\n'
            f'[[package]]\nname = "core"\nversion = "{OLD}"\n'
            'source = "registry+https://example.org"\nchecksum = "keep"\n\n'
            '[[package]]\nname = "sdk"\nversion = "1.0.0"\n'
        )
        (self.root / "Cargo.lock").write_bytes(lock.replace("\n", "\r\n").encode())
        (self.root / "plugin.toml").write_text(f'host_version = ">={OLD}"\n', encoding="utf-8")

    def run_cli(self, *args: str) -> int:
        output = io.StringIO()
        error = io.StringIO()
        with (
            mock.patch.object(bump_version, "ROOT_DIR", self.root),
            contextlib.redirect_stdout(output),
            contextlib.redirect_stderr(error),
        ):
            result = bump_version.main(list(args))
        self.cli_error = error.getvalue()
        return result

    def snapshot(self) -> dict[Path, bytes]:
        return {path: path.read_bytes() for path in self.root.rglob("*") if path.is_file()}

    def init_git(self) -> None:
        bump_version.git(self.root, "init", "--initial-branch=main")
        bump_version.git(self.root, "config", "user.name", "Version Script Test")
        bump_version.git(self.root, "config", "user.email", "version-test@example.invalid")
        bump_version.git(self.root, "config", "commit.gpgsign", "false")
        # Exercise CRLF preservation without inheriting Windows autocrlf settings.
        bump_version.git(self.root, "config", "core.autocrlf", "false")
        hooks = self.root / ".git" / "empty-hooks"
        hooks.mkdir()
        bump_version.git(self.root, "config", "core.hooksPath", str(hooks))
        bump_version.git(self.root, "add", "--", "Cargo.toml", "Cargo.lock", "crates", "plugin.toml")
        bump_version.git(self.root, "commit", "-m", "test: create workspace fixture")

    def test_updates_only_inherited_local_packages_and_dependency_references(self) -> None:
        before = self.snapshot()
        self.assertEqual(self.run_cli(f"v{NEW}"), 0)
        manifest = (self.root / "Cargo.toml").read_bytes()
        self.assertEqual(manifest, before[self.root / "Cargo.toml"].replace(OLD.encode(), NEW.encode()))
        lock = tomllib.loads((self.root / "Cargo.lock").read_text(encoding="utf-8"))
        app, core, external, sdk = lock["package"]
        self.assertEqual((app["version"], core["version"]), (NEW, NEW))
        self.assertEqual(app["dependencies"][0], f"core {NEW}")
        self.assertEqual(app["dependencies"][1], f"core {OLD} (registry+https://example.org)")
        self.assertEqual(external["version"], OLD)
        self.assertEqual(external["checksum"], "keep")
        self.assertEqual(sdk["version"], "1.0.0")
        after = self.snapshot()
        for path in before.keys() - {self.root / "Cargo.toml", self.root / "Cargo.lock"}:
            self.assertEqual(before[path], after[path])
        self.assertNotIn(b"\n", after[self.root / "Cargo.lock"].replace(b"\r\n", b""))

    def test_dry_run_leaves_every_file_unchanged(self) -> None:
        before = self.snapshot()
        self.assertEqual(self.run_cli(NEW, "--dry-run"), 0)
        self.assertEqual(self.snapshot(), before)

    def test_repeated_bump_is_a_noop(self) -> None:
        self.assertEqual(self.run_cli(NEW), 0)
        before = self.snapshot()
        self.assertEqual(self.run_cli(NEW), 0)
        self.assertEqual(self.snapshot(), before)

    def test_accepts_stable_prerelease_and_build_versions(self) -> None:
        for version in ("2.0.0", "3.1.0-rc.1", "3.1.0+build.01"):
            with self.subTest(version=version):
                self.assertEqual(self.run_cli(version), 0)
                manifest = tomllib.loads((self.root / "Cargo.toml").read_text(encoding="utf-8"))
                self.assertEqual(manifest["workspace"]["package"]["version"], version)

    def test_invalid_versions_do_not_write_files(self) -> None:
        before = self.snapshot()
        for version in ("2.0", "02.0.0", "2.0.0-preview.05", "2.0.0-", "2.0.0\nmalicious"):
            with self.subTest(version=version):
                self.assertEqual(self.run_cli(version), 1)
                self.assertEqual(self.snapshot(), before)

    def test_invalid_or_incomplete_lock_does_not_write_manifest(self) -> None:
        lock_path = self.root / "Cargo.lock"
        original = lock_path.read_bytes()
        for broken in (
            original.replace(b'name = "app"', b'name = "missing"'),
            original.replace(OLD.encode(), b"0.0.1"),
            b"invalid TOML!",
        ):
            with self.subTest(lock=broken):
                lock_path.write_bytes(broken)
                before = self.snapshot()
                self.assertEqual(self.run_cli(NEW), 1)
                self.assertEqual(self.snapshot(), before)

    def test_commit_contains_only_version_files_and_preserves_unrelated_staging(self) -> None:
        self.init_git()
        plugin = self.root / "plugin.toml"
        plugin.write_text('host_version = ">=2.0.0"\n', encoding="utf-8")
        bump_version.git(self.root, "add", "--", "plugin.toml")
        staged = bump_version.git(self.root, "diff", "--cached")
        (self.root / "notes.txt").write_text("unfinished work\n", encoding="utf-8")
        self.assertEqual(self.run_cli(NEW, "--commit"), 0, self.cli_error)
        self.assertEqual(
            bump_version.git(self.root, "log", "-1", "--format=%s"),
            f"chore(release): bump version to {NEW}",
        )
        self.assertEqual(
            set(bump_version.git(self.root, "diff-tree", "--no-commit-id", "--name-only", "-r", "HEAD").splitlines()),
            {"Cargo.toml", "Cargo.lock"},
        )
        self.assertEqual(bump_version.git(self.root, "diff", "--cached"), staged)
        self.assertEqual((self.root / "notes.txt").read_text(), "unfinished work\n")
        head = bump_version.git(self.root, "rev-parse", "HEAD")
        self.assertEqual(self.run_cli(NEW, "--commit"), 0)
        self.assertEqual(bump_version.git(self.root, "rev-parse", "HEAD"), head)

    def test_commit_accepts_crlf_and_preserves_configured_whitespace_checks(self) -> None:
        self.init_git()
        bump_version.git(self.root, "config", "core.whitespace", "trailing-space,tab-in-indent")
        self.assertEqual(self.run_cli(NEW, "--commit"), 0, self.cli_error)
        self.assertEqual(
            bump_version.git(self.root, "config", "--get", "core.whitespace"),
            "trailing-space,tab-in-indent",
        )
        for filename in bump_version.VERSION_FILES:
            data = (self.root / filename).read_bytes()
            self.assertIn(b"\r\n", data)
            self.assertNotIn(b"\n", data.replace(b"\r\n", b""))
        # A changed tab-indented version must still fail the configured check.
        manifest = self.root / "Cargo.toml"
        manifest.write_bytes(manifest.read_bytes().replace(b"version = '", b"\tversion = '"))
        bump_version.git(self.root, "add", "--", "Cargo.toml")
        bump_version.git(self.root, "commit", "-m", "test: add configured whitespace fixture")
        head = bump_version.git(self.root, "rev-parse", "HEAD")
        self.assertEqual(self.run_cli("2.0.0-preview.6", "--commit"), 1)
        self.assertIn("tab in indent", self.cli_error)
        self.assertEqual(bump_version.git(self.root, "rev-parse", "HEAD"), head)

    def test_commit_dry_run_preserves_files_head_and_index(self) -> None:
        self.init_git()
        before = self.snapshot()
        self.assertEqual(self.run_cli(NEW, "--commit", "--dry-run"), 0)
        self.assertEqual(self.snapshot(), before)

    def test_commit_rejects_existing_staged_and_unstaged_version_edits(self) -> None:
        self.init_git()
        manifest = self.root / "Cargo.toml"
        manifest.write_bytes(manifest.read_bytes() + b"\r\n# unrelated edit\r\n")
        for staged in (False, True):
            with self.subTest(staged=staged):
                if staged:
                    bump_version.git(self.root, "add", "--", "Cargo.toml")
                before = self.snapshot()
                self.assertEqual(self.run_cli(NEW, "--commit"), 1)
                self.assertEqual(self.snapshot(), before)

    def test_commit_rejects_detached_head_before_writing(self) -> None:
        self.init_git()
        bump_version.git(self.root, "checkout", "--detach")
        before = self.snapshot()
        self.assertEqual(self.run_cli(NEW, "--commit"), 1)
        self.assertEqual(self.snapshot(), before)

    def test_commit_rejects_active_merge_before_writing(self) -> None:
        self.init_git()
        (self.root / ".git" / "MERGE_HEAD").write_text(
            bump_version.git(self.root, "rev-parse", "HEAD") + "\n", encoding="utf-8"
        )
        before = self.snapshot()
        self.assertEqual(self.run_cli(NEW, "--commit"), 1)
        self.assertEqual(self.snapshot(), before)

    def test_failed_commit_leaves_version_edits_and_staging_for_review(self) -> None:
        self.init_git()
        head = bump_version.git(self.root, "rev-parse", "HEAD")
        real_git = bump_version.git

        def fail_commit(root: Path, *args: str) -> str:
            if args[0] == "commit":
                raise ValueError("commit hook rejected the change")
            return real_git(root, *args)

        with mock.patch.object(bump_version, "git", side_effect=fail_commit):
            self.assertEqual(self.run_cli(NEW, "--commit"), 1)
        self.assertEqual(bump_version.git(self.root, "rev-parse", "HEAD"), head)
        self.assertEqual(bump_version.git(self.root, "diff", "--cached"), "")
        manifest = tomllib.loads((self.root / "Cargo.toml").read_text(encoding="utf-8"))
        self.assertEqual(manifest["workspace"]["package"]["version"], NEW)

    def test_commit_requires_a_git_repository_without_writing(self) -> None:
        before = self.snapshot()
        self.assertEqual(self.run_cli(NEW, "--commit"), 1)
        self.assertEqual(self.snapshot(), before)


if __name__ == "__main__":
    unittest.main()
