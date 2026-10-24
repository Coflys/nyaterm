# Release version

Use Python 3.11 or newer to preview and apply a release version from the repository root:

```sh
python scripts/release/bump_version.py 2.0.0-preview.5 --dry-run
python scripts/release/bump_version.py 2.0.0-preview.5
git diff -- Cargo.toml Cargo.lock

# Update and commit the release version in one step:
python scripts/release/bump_version.py 2.0.0-preview.5 --commit

# Preview both the edits and the commit message:
python scripts/release/bump_version.py 2.0.0-preview.5 --commit --dry-run
```

Stable versions such as `2.0.0` and a leading `v` are also accepted. The script
validates SemVer and both files before writing. It preserves comments and line
endings and works without invoking Cargo, accessing the network, or updating
dependency resolutions. Repeating the same version is a no-op.

`--commit` creates `chore(release): bump version to <version>` containing only
`Cargo.toml` and `Cargo.lock`. Unrelated staged and unstaged changes are preserved.
Both version files must be tracked and clean before running this option, so
existing manifest or dependency edits cannot accidentally enter the release
commit. A branch, valid Git identity, and no conflicts or unfinished Git
operations are required. These checks run before editing files. If a commit
hook or Git commit fails, the version edits remain for review and manual commit.
An unchanged version creates no commit.

The script updates `[workspace.package].version` in the root `Cargo.toml` and the
local packages in `Cargo.lock` whose manifests use `version.workspace = true`.
Those member manifests need no edits. Application version strings use
`CARGO_PKG_VERSION`; native packaging reads the workspace version and generates
platform version metadata and release manifests automatically.

Windows application and installer numeric versions use four 16-bit components:
`major.minor.patch.revision`. The revision is the final prerelease identifier if
it is numeric, otherwise zero: `2.0.0-preview.9` becomes `2.0.0.9`, while `2.0.0`
and `2.0.0-beta` become `2.0.0.0`. Build metadata is ignored for numeric versions.
Components above 65535 fail the Windows build or packaging rather than truncate.
The application's product version retains the full Cargo SemVer string.

Other version values have separate meanings and remain unchanged:

- `nyaterm-otp`, `nyaterm-plugin-api`, and plugin guest packages have independent
  package versions.
- Plugin `host_version` constraints record the minimum compatible host release;
  example manifests, documentation, and compatibility fixtures retain that floor.
- The docs site's `package.json` and the pinned ConPTY package have independent
  versions.

Commit the version changes before creating the matching `v<version>` release
tag. The release workflow requires the tag/input version to match the Cargo
workspace version. The script can create a local commit with `--commit`; tags,
pushes, and release publishing remain separate steps.
