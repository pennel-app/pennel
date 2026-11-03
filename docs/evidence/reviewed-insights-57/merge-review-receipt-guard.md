# Receipt-bound note body guard

The merge review of PR #132 found that the common desktop/iPhone notes writer
discarded the boolean returned by `validate_insight_note_write`. A pending
insertion receipt could therefore authorize a different note body despite binding
the intended body by hash.

The writer now rejects a false match before `notes.md` is written or the receipt
is acknowledged. The receipt validator API and persistence format are unchanged.
Ordinary note saving remains independent of the review store.

## Regression and verification

The new engine regression first failed because the old writer accepted an
unprepared body. It now passes all four combinations of desktop/iPhone entry
point and pending receipt with either original notes or intended notes already on
disk after interrupted acknowledgement. Each rejection preserves the notes and
`insights.json` bytes. The same receipt then accepts the intended body, acknowledges
it and supports an idempotent replay without rewriting the review state.

Commands run in the review worktree:

```sh
scripts/with-toolchain.sh cargo test --manifest-path core/Cargo.toml --no-default-features --lib meeting_insight_engine::tests::receipt_bound_note_writes_reject_unprepared_text_without_mutating_saved_state -- --exact
scripts/with-toolchain.sh cargo test --manifest-path core/Cargo.toml --lib meeting_insight_engine::tests::
scripts/with-toolchain.sh cargo test --manifest-path core/Cargo.toml --no-default-features --lib meeting_insight_engine::tests::
scripts/with-toolchain.sh cargo clippy --manifest-path core/Cargo.toml --lib --tests -- -D warnings
scripts/with-toolchain.sh cargo clippy --manifest-path core/Cargo.toml --no-default-features --lib --tests -- -D warnings
scripts/with-toolchain.sh cargo fmt --manifest-path core/Cargo.toml -- --check
git diff --check
```

The targeted regression passed, and both pipeline suites passed 29 tests with no
failures. Strict default/lean Clippy, formatting and whitespace checks passed.
Existing pipeline cases also cover concurrent note edits, unreadable notes,
ordinary saves with an unsupported review sidecar and optional acknowledgement
I/O failure. Independent correctness and architecture rechecks cleared the patch.
Logs remain under `build/merge-review-127-132/receipt-guard/`.

Full joined verification and current-head CI follow integration with the separately
reviewed recovery-audio fix. This narrow check does not establish physical-device,
model-quality, real-call or deployment acceptance. No provider request, model
download or qualification change was made.
