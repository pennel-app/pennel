# Physical recovery-audio completion — merge review follow-up

Prepared for #22 / PR #128 and #99 / PR #129 on 3 November 2026. This closes a
merge-review data-loss finding; it does not establish real-call, acoustic-quality,
hardware, resource, signed-device or deployment acceptance.

## Failure and resulting behavior

The previous cleanup selected recovery WAVs using the displayed `Segment.speaker`
and utterance start. Person attribution can relabel a microphone row as `Others`,
and verified acoustic splits can start after the parent capture chunk. Cleanup
could therefore remove another physical track's failed WAV, or retain and later
resurrect an already completed parent chunk.

Canonical metadata now carries immutable original capture-lane, basename,
start/end, WAV SHA-256 and disposition. Completion is recorded in the same durable
transcript/metadata transaction and survives split lineage. The same proof controls
recovery suppression and unlinking. Neither person labels nor acoustic subranges
authorize deletion. Failed or unverified files remain local; unresolved legacy
overlaps are not automatically sent for recognition.

Discovery reads only bounded fixed headers for pending/unverified WAVs. Only a
matching completed candidate or explicit retry hashes payload bytes. Payload
reads themselves are capped at one supported capture chunk plus one sentinel byte,
even if a file grows between header validation and reading. Symlink files,
symlink/invalid parent folders, changed bytes and malformed headers cannot authorize
cleanup or recognition.

Empty successful canonical rows are durable privacy/completion dispositions. All
normal writes and replacements preserve them even without local PCM: metadata is
portable, while recovery audio is not synced. A peer saving its visible empty view
cannot erase the origin's disposition. Local cleanup bookkeeping separately uses
no-follow entry checks; a dangling link is not treated as verified absence.

An idempotent-write fallback compares full committed canonical rows, including
hidden completion proof. It cannot acknowledge an old empty public projection as
proof that a new private disposition was saved.

## Compatibility

| Metadata schema | Capability |
| --- | --- |
| 1 | Existing settled utterances |
| 2 | Existing provisional attribution |
| 3 | Original recovery provenance, settled attribution |
| 4 | Original recovery provenance and provisional attribution |

The #128 backport supports schemas 1/3 and preserves 2/4 read-only. The #129 reader
supports 1–4. New serde types do not add UniFFI record fields or change native API
signatures. Unsupported or ambiguous provenance never authorizes deletion.

## Regression evidence

Preserved logs are under `build/merge-audio-cleanup/` in the original checkout.

- `red-cross-source-exact.log`: original cleanup deletes the failed system WAV
  after microphone person relabeling. The corrected test preserves both unproven
  originals; proven-source tests remove only the completed physical chunk.
- `live-split-commit.log` and `schema-journal-boundaries.log`: live/offline splits,
  first timed span after the parent start, interrupted commits and all three
  journal boundaries preserve the same source proof. Failed writes retain both
  tracks before any cleanup.
- `red-private-marker-retention.log` / `green-private-marker-retention.log`:
  malformed headers and dangling file/directory entries cannot erase hidden
  dispositions on a subsequent save.
- `red-raw-completion-cas.log` / `raw-canonical-replay.log`: stale public-view
  equivalence is rejected; a genuinely identical complete durable snapshot is
  still acknowledged.
- `red-cleanup-absence.log` / `green-cleanup-absence.log`: dangling entries stay in
  pending cleanup bookkeeping.
- `red-legacy-disposition.log` / `green-legacy-disposition.log`, and the explicit
  lane ambiguity regression: knowing a lane without the original chunk proof
  cannot promote unverified overlapping audio into a recognition request.
- `red-portable-private-disposition-corrected.log` /
  `green-portable-private-disposition.log`: two stores exchange root metadata only;
  the peer has no recovery directory, saves `[]`, and syncs back. The origin has no
  failed/private resurrection, issues zero mock STT requests, and removes its exact
  completed WAV only after hash verification. The earlier incorrectly initialized
  fixture is retained separately and is not claimed as the policy regression.
- Payload-byte instrumentation proves repeated pending/unverified listings read
  zero PCM bytes. Complete candidates and explicit retry read/hash actual bytes;
  the growing-reader regression stops at the fixed cap without a large fixture.

Independent correctness and architecture reviews inspect the frozen production
diff and preservation follow-ups. Final commands/results are recorded with the
commit handoff. No models, actual provider calls, signing identities, credentials
or production settings were used or changed by this follow-up.

## Final local verification

The final frozen production patch is
`c618c90149adcf1f7855e7821b0b974a3dbd4581cc7672368cc67993f16028d4`.
Both independent lanes returned CLEAR after the portable-marker regression.

| Command | Result |
| --- | --- |
| `scripts/with-toolchain.sh cargo test --manifest-path core/Cargo.toml --lib` | 904 passed, 0 failed, 7 existing ignored |
| Same command with `--no-default-features` | 887 passed, 0 failed, 5 existing ignored |
| `cargo clippy --manifest-path core/Cargo.toml --lib --tests -- -D warnings` through the toolchain wrapper | Passed |
| Same strict Clippy command with `--no-default-features` | Passed |
| `cargo fmt --manifest-path core/Cargo.toml -- --check` through the wrapper; `git diff --check` | Passed |
| `make core` | Release library built |

Exact final logs are `core-default-v7.log`, `core-lean-v7.log`,
`clippy-default-v7.log`, `clippy-lean-v7.log` and `core-build-v7.log` in the retained
build folder above. Filtered/targeted suites overlap the full suites and are not
additional test totals. The metadata-only peer round trip makes zero mock STT
requests; no real provider was contacted. The native ABI and frontend code did not
change. Native/platform, signed-device and actual audio acceptance are separate
from this core regression evidence.

## Replay onto the fixed speaker-turn base

The #129 phase was replayed onto fixed #128 commit
`5da3212b7af116b38d1bb4cc71ec697cb5e8ded3`. A backup ref retains the reviewed
`12a2bce378ed03107b4e1fc373a3e01584b10e8e` implementation. All six touched Rust
production prefixes are byte-identical to that reviewed implementation; engine,
library exports and retranscription files are identical in full. The remaining
differences retain #128's canonical source-cleanup and pre-live timed-split/reopen
regressions, plus its conservative future-format test.

The future-format test appropriately checks schemas 5/9000 after #129 introduces
support for 2/4. Separate capability tests continue to reject recovery proof under
old schemas and provisional attribution under settled-only schema 3. The #128
branch itself keeps 2/4 read-only.

The replay passes 32 recovery-filtered tests, the carried #128 offline split/commit
case and the future-format case. Full suites pass **907 default + 7 ignored** and
**890 lean + 5 ignored**, with zero failures; strict Clippy and formatting also pass.
Logs use the `restack-` prefix in the retained build folder. The production audit is
`restack-production-audit.json`; no provider, model, settings or signing operation
was performed during replay. No native ABI or application behavior changed while
resolving the replay conflicts.
