# Live-worker fixture admission — 3 November 2026

[PR #132](https://github.com/pennel-app/pennel/pull/132), candidate `a006e5c`, failed its [Linux core job](https://github.com/pennel-app/pennel/actions/runs/41672788946/job/126046219296): `retired_native_worker_keeps_its_slot_and_offline_work_has_priority` reached the generic ten-second checkpoint timeout. Two later session tests then encountered the poisoned test mutex. The result was 972 passed, three failed and seven existing ignored tests. The original log does not retain the calling checkpoint.

## Controlled failure and repair

One hundred isolated executions and a full parallel #99 suite passed without reproducing the CI failure. A controlled retained predecessor then reproduced a specific admission race: after logical Stop, the previous native worker still owned its physical slot. The next worker was pending, so its one-shot feed was correctly discarded (`admitted=false`, `ring_samples=0`, `pending_samples=0`). Releasing the predecessor allowed admission, but could not restore that discarded input. The test failed at its initial inference-entry wait after 10.05 seconds, with the exact caller and backtrace recorded.

The existing test now retains that predecessor as a deterministic regression. It asserts that pre-admission input is discarded, waits for actual admission, then acknowledges accepted input before waiting for inference. Retrying the same synthetic audio range cannot schedule another cadence. Every wait has a named phase and caller location; the ten-second checkpoint bound is unchanged.

The original physical-retirement, offline-priority, stale-result and exact-call assertions remain. Shared counters additionally verify peak physical occupancy of one across all synthetic native calls and the offline permit, then zero active calls after cleanup. Cleanup releases every fake latch, stops all owned workers, joins offline work and waits for finished workers plus the vacant global lifetime slot; a failed normal cleanup fails the test. Unwinding also releases and stops every owned handle with the same bounded cleanup.

All changes are inside `cfg(test)`. Production source before the test module is byte-identical to the prior commit. No admission, priority, capacity, cancellation, timeout or PCM limit in the application changed. The primary failure and poisoned mutex are not suppressed. Independent narrow review approved the causal synchronization, cleanup and retained assertions before commit.

## Verification

| Check | Result |
|---|---|
| Controlled retained-predecessor case | Failed before repair; passed after repair in 0.10 seconds |
| Full default core suite | 881 passed, seven existing ignored |
| Full lean core suite | 864 passed, five existing ignored |
| Strict default and lean Clippy | Passed with warnings denied |
| Core formatting and diff check | Passed |

From the repository root, using the existing toolchain and automatic Rust storage guard:

```sh
scripts/with-toolchain.sh cargo test --manifest-path core/Cargo.toml --lib \
  live_speakers::tests::retired_native_worker_keeps_its_slot_and_offline_work_has_priority \
  -- --exact --nocapture
scripts/with-toolchain.sh cargo test --manifest-path core/Cargo.toml --lib
scripts/with-toolchain.sh cargo test --manifest-path core/Cargo.toml --no-default-features --lib
scripts/with-toolchain.sh cargo clippy --manifest-path core/Cargo.toml --lib --tests -- -D warnings
scripts/with-toolchain.sh cargo clippy --manifest-path core/Cargo.toml --no-default-features --lib --tests -- -D warnings
scripts/with-toolchain.sh cargo fmt --manifest-path core/Cargo.toml -- --check
git diff --check
```

Ignored logs remain under `build/notetaker-2026-11-03/ci/`: `pr132-a006e5c-job126046219296.log`, `live-priority-retained-predecessor-{red,green}.log`, `live-priority-{default,lean}-green.log`, `live-priority-{default,lean}-clippy.log`, plus isolated/full control logs. Counts overlap and are not additional totals. No model or provider ran.

Hosted CI on the repaired commit is pending. [Real-call, acoustic quality, hardware/resource and installed-platform acceptance](../live-speakers-99.md#remaining-acceptance) remains open. No release or deployment occurred.
