# Managed auth/content deadline fixture — CI follow-up

The [PR #132 hosted lean job](https://github.com/pennel-app/pennel/actions/runs/41678611439/job/126066003481)
at `8ba325c7ac3ef97581eef7959fcd279db492fa14` passed 956 tests and failed the
managed auth/content deadline fixture. Its 120 ms auth delay and 180 ms content
delay shared a 220 ms deadline. Under hosted scheduling the valid deadline could
expire after auth but before content dispatch, while the test required one content
POST. This was a fixture ordering assumption. `ManagedInference::send`, session
checks and transport already carry the same absolute deadline.

The append-only #98 repair changes only `core/src/inference_tests.rs`, which is
included under `cfg(test)`, and this evidence. Production transport, clocks,
deadlines, priorities, model/provider configuration and qualification gates are
unchanged. No native, frontend, bindings or Release rebuild is needed for this
test-only change.

## Controlled proof

- Explicit entry/release channels observe auth and authenticated content. Both
  exact request bodies, the content authorization header and one request per phase
  remain mandatory. A zero-content-request run cannot pass.
- Auth deliberately consumes half of a generous 10-second **test** budget. Valid
  content remains withheld while the real `ManagedInference::send` must return a
  network deadline error between the original deadline and its one-second test
  observation allowance. Early auth/socket errors cannot satisfy that criterion.
- A fresh session and a deliberately wrong test-only per-phase-reset path are the
  negative control. That path must obtain the valid content JSON after the original
  boundary but before its earliest fresh deadline. Its success proves that the
  shared-budget assertion detects resetting the deadline, rather than accepting an
  arbitrary fixture/auth error or merely enlarging timeouts.
- Callback release-on-drop runs before scoped-worker and mock teardown. Thirty-second
  handshake watchdogs bound broken fixtures. This ordering also avoids retaining
  Mockito's state lock during panic cleanup.

For the controlled red run, the primary assertion was temporarily routed through
the reset strategy in the test file. It failed in **11.01 seconds**, with both
POSTs observed and valid JSON returned after the 10-second original deadline and
before the 15-second reset boundary. Restoring the unchanged production path made
all eight managed transport tests pass, including the permanent reset control.
Production source was never modified for this experiment.

The [machine receipt](deadline-fixture-ci-follow-up.json) records source/log hashes,
commands and full default/lean results. Bounded independent static review approved
the request-entry proof, cumulative-budget oracle, fresh negative-control session,
cleanup and unchanged production deadline. Hosted CI and #57's joined acceptance
remain separate evidence; no real model/provider call or resource qualification is
established by these loopback fixtures.
