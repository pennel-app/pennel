# Recovery audio invariant repair — PR #128

This merge-review repair builds on `0a6a841a4c12d53db86cee516714adb0636b29bb` for issue #22 and is replayed into the live-attribution phase #99. It repairs recovery handling without adding live/provisional attribution to this phase.

A displayed speaker or acoustic word start cannot identify an original recovery WAV. Cleanup previously reconstructed a filename from those mutable fields, allowing a relabelled microphone success to delete another capture lane's failed audio. A retained successful parent could also reappear as failed after timed splits.

Canonical utterances now carry optional original capture-lane, basename, input range, WAV SHA-256 and disposition. Splits retain that proof. Cleanup and recovery suppression share the same complete-proof check and verify actual bytes. Unknown, changed, overlapping unbound, malformed and symlink artifacts remain retained; ambiguous legacy audio stays unverified and is not automatically submitted for recognition. The capture lane is independent of a person label and can represent system or room input.

Successful empty/private completion markers are portable state and survive normal writes and retranscription, including metadata-only round trips through a device without the original PCM. Local absence cannot prove that another device cleaned up. Persistence replay compares the complete committed rows, including hidden proofs. Cleanup bookkeeping treats uncertain or dangling paths as retained.

Discovery reads fixed headers, not pending PCM payloads. Completed candidates and explicit retries verify bytes using a reader capped at the 31-second input limit plus one byte; a growing-reader test proves the bound without allocating a huge fixture.

This phase reads schemas **1 and 3**, writes schema 3 when recovery proof is present, and preserves schemas **2 and 4** read-only. Those later formats may carry provisional-attribution semantics that this phase cannot interpret. The added types are Rust re-exports only; no UniFFI record or native signature changes.

## Verification

- The original cross-lane deletion regression failed before the repair (`red-cross-source.log`) and passes after it.
- Metadata-only peer rewrite failed with a lost private marker (0 versus 1); the repaired case passes with no failed resurrection, zero provider requests and hash-verified origin cleanup.
- Focused cases cover both physical lanes, same-source overlap, nonzero first-word boundaries and splits, commit interruption/reopen, changed bytes, private/empty markers, malformed/truncated headers, symlink retention, bounded payload reads, sticky unverified state, and full raw persistence replay.
- Schema 1/3 journal recovery passes all six interruption combinations. Schema 2/4 and unknown future metadata remain unchanged/read-only.
- Final full core: **870 default passed / 853 lean passed**, with 7/5 existing ignored and zero failures. Strict default/lean Clippy, formatting and whitespace checks passed.
- Both independent merge-review lanes cleared the frozen production patch; no provider/model, signing identity, credentials or settings were changed.

Exact commands use the existing guarded Rust toolchain:

```sh
scripts/with-toolchain.sh cargo test --manifest-path core/Cargo.toml --lib
scripts/with-toolchain.sh cargo test --manifest-path core/Cargo.toml --no-default-features --lib
scripts/with-toolchain.sh cargo clippy --manifest-path core/Cargo.toml --lib --tests -- -D warnings
scripts/with-toolchain.sh cargo clippy --manifest-path core/Cargo.toml --no-default-features --lib --tests -- -D warnings
scripts/with-toolchain.sh cargo fmt --manifest-path core/Cargo.toml -- --check
git diff --check
```

Logs, controlled red evidence, patch snapshots and production-prefix comparisons are retained under `build/merge-audio-backport/`. Rejected patch hunks were moved there as adaptation evidence; only the intended source backport remains in the working tree. No source worktree, product, symbol or recovery data was deleted. Hosted CI, real-call/acoustic quality, resource, physical-device and installed-platform acceptance remain separate; keep issue #22 open for its unmet acceptance criteria.
