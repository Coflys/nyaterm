# Cloud sync protocol and compatibility

Cloud sync transfers complete portable configuration snapshots. It excludes
device-local presentation preferences and command history. It does not perform
entity merges or provide a general backup history browser.

## Encryption

New encrypted snapshots have this binary envelope (integers are little-endian):

| Offset | Contents |
| --- | --- |
| 0 | Eight-byte magic `NYASNAP\0` |
| 8 | Envelope version `2` |
| 9 | KDF identifier `1` (Argon2id, algorithm version 0x13) |
| 10 | Memory cost, u32: 19456 KiB |
| 14 | Time cost, u32: 2 |
| 18 | Parallelism, u32: 1 |
| 22 | Random 16-byte salt |
| 38 | Random 12-byte AES-GCM nonce |
| 50 | AES-256-GCM ciphertext followed by its 16-byte tag |

The entire 50-byte header is authenticated as AAD. Readers validate the version
and the supported cost profile before allocating KDF memory. They do not accept
arbitrary remote-controlled memory or iteration costs. Derived key buffers are
zeroized. Password trimming remains compatible with historical behavior.

Readers retain both historical `nonce || ciphertext` key domains, NyaTerm and
Dragonfly. Old bytes are never rewritten just by reading. An actual upload or
explicit forced push writes the new envelope; unmodified-content checks do not
silently re-encrypt existing data. Unsupported future envelopes are reported as
unsupported, including through the decoder helper, and do not enable forced
overwrite as a corruption-recovery shortcut.

All devices sharing a namespace must be upgraded before publishing new-format
snapshots. Legacy readers cannot decrypt them. This is a read-compatible
migration, not a promise of old-client write interoperability. Keep an independent
local `.nya` backup when upgrading.

## Data and change detection

The redb snapshot schema remains version 3, and historical tables, document
keys, payload hashes, ZIP containers, and optional entity integrity hashes stay
readable. The codec uses a bounded in-memory redb backend instead of temporary
plaintext files. File-backed redb interoperability is tested in both directions.

`last_synced_content_hash` is a new optional local state field. Its
`entities-v1:` hash covers the complete sorted entity map, including VNC trust
records and unknown entities. `last_synced_payload_hash` retains its old meaning
for compatibility; it is no longer sufficient evidence for change detection.
An absent new baseline is treated conservatively: equal complete snapshots
establish the baseline, while differing snapshots cannot silently auto-pull over
data omitted by an old hash.

## Publication and concurrency

The remote layout is:

* `sync/snapshots/<revision>.redb.enc`: immutable encrypted data.
* `sync/heads/<sha256(device_id)>.json`: independently published device commit.
* `sync/heads/<sha256(device_id)>.pending.json`: protected staged revision.
* `sync/latest.redb`: best-effort compatibility pointer.
* `sync/current.redb.enc`: best-effort compatibility/recovery copy.

Each device has one persisted, unique device ID and one serialized sync writer.
Copying a live database to another device must not be used as a way of cloning
that writer identity; use portable import instead. The protocol assumes atomic
replacement of individual objects and eventually visible reads/listings, not an
atomic transaction across objects. Google Drive duplicate path names fail
closed rather than selecting an arbitrary file or folder.

Snapshot metadata and its pointer carry an optional `sync_clock` with per-device
monotonic counters. A clock supersedes another only if every counter is at least
as large and one is larger. Legacy pointers are linked through `legacy_revision`.
The clock is inside the authenticated snapshot and is checked against the head
when that snapshot is decoded. Wall-clock time is not a conflict winner.

A push registers its staged revision, uploads and verifies the immutable bytes,
then publishes its own device head. Cancellation is allowed before publication;
once publication begins the job finishes and reports the outcome. Failure to
refresh the shared compatibility objects does not undo an independently
published commit. Staging does not overwrite the previously committed head.

Two devices can both report successful publication. Their independent heads
retain both branches even if the shared compatibility pointer is overwritten.
The next check that can observe both branches reports a conflict. This is eventual
conflict detection, not linearizable compare-and-swap. Tests force both writers
past the historical check-before-write window before permitting publication.
With delayed listings, a conflict may appear on a later check.

An explicit local/cloud choice publishes a new causal commit covering the
observed branches. Cloud choice uses the branch read when the operation starts.
Unobserved later writes remain concurrent and produce a new conflict. Explicit
recovery also creates a new revision rather than rewinding another device's
head. Non-forced checks and pulls reject causal ancestors of the locally synced
baseline so a stale remote response cannot roll the device back.
They also reject a remote branch concurrent with that baseline even when a
delayed listing omits this device's own committed head. Explicit cloud choice
then publishes a new resolving revision using both causal histories.

## Local apply and efficiency

An automatic check carries its exact local snapshot, decoded remote snapshot,
and checked baseline into the subsequent push/pull. It never rebuilds a baseline
after network I/O and mistakes intervening saves for synchronized content.

The production store checks the expected complete content hash, writes a safety
backup, applies the incoming snapshot transactionally, rebuilds the normalized
baseline, and saves it in one serialized store job. No application save can run
between these steps. A crash between the data transaction and baseline write
leaves a conservative old baseline, causing revalidation/conflict rather than
marking unrelated edits synchronized. Even forced pulls reject local changes
made after their initial snapshot was captured.

Without retries or maintenance, a checked automatic push downloads the previous
snapshot once and the uploaded snapshot once for readback validation. It reuses
the uploaded ciphertext for the compatibility copy. Metadata calls and provider
path lookup calls are additional. Pulls reuse the checked remote snapshot and do
not upload a compatibility copy as a side effect. The transfer-count regression
test also confirms that a later local edit remains dirty.

Maintenance authenticates committed heads once per pass, skips protected snapshots
in the candidate scan, retains five recent eligible historical snapshots and a
24-hour grace period, and rechecks heads before deletion. A versioned candidate
must also be a strict causal ancestor of an individually authenticated commit;
absence from a delayed listing never proves that a branch is disposable.
Capacity-limited snippet cleanup may collect additional
verified old history, but never active, staged, foreign, or unreadable objects.
Device commit and staging files count toward provider file limits; if protected
data fills a snippet, synchronization fails with a capacity error instead of
discarding a branch. Existing `backups/` data is outside this cleanup policy.

## Validation

Regression coverage includes legacy envelope migration, wrong passwords,
tampered headers/ciphertext, bounded KDF parameters, complete-entity change
detection, saves during downloads and before apply, independently published
concurrent heads, explicit resolution in either direction, stale remote views,
staged/committed snapshot GC protection, compatibility-write failures, cancellation,
readback delays, and redb file/memory codec compatibility. Native provider behavior
still depends on each service honoring successful object writes and eventually
consistent listing semantics; no real cloud credentials are used by unit tests.


## Credentials, diagnostics and scheduling

HTTP failures expose only a controlled provider/operation, status and error
classification. Request URLs, query parameters, authorization headers and raw
response bodies never become user-visible or logged diagnostics. Capacity
classification retains the public `gist_capacity` code. Numeric and HTTP-date
`Retry-After` values extend retry backoff; authentication failures without a
rate-limit hint pause background work until settings change.

Google Drive, OneDrive and Aliyun Drive persist refreshed access tokens and any
returned refresh token through the serialized store before another authenticated
request. Missing refresh tokens retain the previous value. The update compares
client identity and both previous tokens, updates only credentials, and uses the
existing encrypted storage/unknown-field merge. Stale updates are rejected and
save failures stop the operation. Subsequent sync failure or cancellation does
not roll back a completed credential save. Settings drafts rebase unedited token
fields; explicit credential edits retain ordinary conflict checks.

All providers, including GitHub Gist, share startup checks (when enabled),
15-minute checks, focus checks throttled to 120 seconds, change debounce, safe
pull deferral and failure backoff. Active sessions and unsaved drafts continue to
block automatic application. One process coordinator records actual transfers,
conflicts, failures and successful recovery before notifying windows. Busy,
disabled, debounce and unchanged checks do not add history. Identical
classification/trigger/revision failures are deduplicated for 15 minutes; history
I/O failure does not alter the sync outcome.

## Conflict previews and device labels

Snapshot metadata may include `device_name` inside the encrypted payload. The
UUID remains the device identity. Older snapshots omit the field and display a
short UUID. Renaming takes effect on the next actual publication and does not
change the entity content hash or trigger a name-only upload. Snapshot schema
version, encryption envelope, payload/content hash definitions, tables and keys
are unchanged.

A conflict preview lists every maximal concurrent revision, device, timestamp,
readability and counts relative to the local content. Known collections use
stable record IDs; settings show categories and unknown entities only a change
marker. Previews never return record names, secret values, notes or command text.
Exactly one readable branch is selected automatically; multiple readable branches
require an explicit choice. Choosing local resolves only the displayed observed
branches, and choosing cloud pins the selected immutable revision.

Resolution requests carry the observed maximal revision set and preview local
content hash. They revalidate at execution and before publication. Changed
branches, missing/unreadable selected snapshots and changed local content refresh
the preview instead of selecting a substitute. A new commit arriving after the
publication check remains concurrent and is reported on the next check. Existing
master-password guards, checked local apply, session restrictions, safety backup
and causal resolution commits remain in force. There is no field-level merge.


## Bounded checks and application-body traffic

The desktop process shares an in-memory I/O runtime across windows and provider
instances. S3 conditionally reads `latest.redb` and device heads with opaque
ETags. Every effective check still lists device heads: an unchanged compatibility
pointer does not prove the absence of a concurrent device commit. Fresh LIST
validators can reuse unchanged head bytes. Missing validators require reads;
unsupported conditionals fall back to GET without bypassing auth or rate limits.
Metadata uses a 16 MiB / 1,024-entry LRU. Writes and deletes invalidate before
I/O, so upload readback cannot be satisfied by pre-write cached bytes.

All providers can reuse authenticated snapshot summaries for automatic checks
for at most 24 hours. Receipts match the complete pointer, including causal clock;
only a successful full validation advances their timestamp. Changed pointers,
remote configuration, credentials, master passwords, cold startup or cache
eviction require full reads. Receipts and secret-free conflict differences are
memory-only; decrypted entity bodies are not retained. Conflict summaries also
match the full branch set and local content hash. Unreadable branches and failed
validations never become successful cache entries. Actual push, pull, recovery
and conflict confirmation still validate snapshots; an already decoded snapshot
from the same check can be reused. GC retains its full authentication and
pre-deletion checks, daily throttle and existing retention policy.

Providers without conditional metadata capabilities still transfer their pointer
and directory API responses. Gist/Snippet container responses can include file
contents, so avoiding an explicit snapshot read does not imply that the provider
transfers metadata alone. The S3 traffic fixture does not predict those APIs.

Pure local-change notifications whose entity hash equals the synchronized
baseline can skip network I/O when no remote state is pending. Startup, periodic,
settings and retry requests are not discarded by local notifications. Periodic
inspection still runs during continuous changes while upload debounce remains
in force. Successful device-head inspections, including manual synchronization,
reset the 15-minute timer and suppress focus checks for 120 seconds; connection
tests and local no-ops do not count as inspections. Disabling startup checks
continues to leave periodic checks enabled.

Controlled operation summaries report requests, submitted upload-body bytes,
received download-body bytes (including error bodies and retries), 304 responses,
cache hits and full-validation attempts. All providers count at their HTTP
boundary, including authentication retries and consumed error bodies. Upload
bytes are the buffered body submitted for each attempt; download bytes include
partial reads before a failure. Statistics omit headers, TLS and billing-specific
accounting.
No URLs, paths, ETags, credentials or entity data appear in these summaries.

The deterministic regression models three devices, 1 MiB snapshots, the existing
1,056,768-byte pointer, 64 checks/day for 30 days, and unchanged data. Warm checks
receive no object bodies until daily validation; modeled object-body downloads
must decrease by at least 95%. This is a simulation, not a real-provider billing
measurement. No remote filenames, serialization/encryption formats, database
keys or persisted sync-state fields change for this optimization.
