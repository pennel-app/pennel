# Production-bound finished-meeting evaluation

The `evaluation` feature adds `finished_meeting_export` and `finished_meeting_replay`
to the existing JSONL `pennel-eval` bridge. Both operations are offline and accept
authored fixture content only. They share production Suggestions preparation,
wire encoding, raw parsing, identity comparison and candidate decoding. Neither
operation creates admission, changes the qualification registry or reads a user's
meeting library/credentials.

Build the bridge through the repository toolchain wrapper:

```sh
scripts/with-toolchain.sh cargo build --manifest-path core/Cargo.toml --no-default-features --features evaluation --bin pennel-eval
```

Freeze the three invented development-only language fixtures. Use the actual
bridge path printed by Cargo if the workspace selects another target directory.

```sh
python3 evaluations/decision-models/finished.py export \
  --bridge core/target/debug/pennel-eval \
  --fixtures evaluations/decision-models/fixtures/finished-development.json \
  --configuration evaluations/decision-models/configs/finished-local.example.json \
  --output build/finished-freeze.json
```

The freeze includes the exact Rust wire-body strings and source mappings, current
production fingerprints, binary/source/protocol/runner hashes, language/family
split, and optional explicitly pinned `expected_actual`. Related translations and
prefixes must share one split. Existing incremental results remain historical and
are not relabelled as this production contract.

Only after the approved official runtime is acquired and ready, opt into the
bounded local transport:

```sh
python3 evaluations/decision-models/finished.py pilot \
  --bridge core/target/debug/pennel-eval \
  --freeze build/finished-freeze.json \
  --output build/finished-pilot.json \
  --execute-local --max-seconds 1800
```

The pilot accepts only the explicit local-Decider loopback configuration, sends
exported bytes unchanged, disables proxies/redirects/retries/fallbacks, and keeps
one absolute 30-second deadline over all serial windows in each fixture. Its
1,800-second total elapsed clock starts after the runtime is ready. A first
provider, parser or identity blocker stops the run before other language trials.
HTTP runs in an owned one-request subprocess terminated at the absolute deadline,
so trickled headers/body cannot extend a socket inactivity timeout indefinitely.
Received response prefixes and status are retained on termination; remote model
completion is not inferred from closing the local HTTP worker. Rust bridge
subprocesses receive the same remaining deadline for validation, preparation and
replay. Mid-run preparation/replay failure returns a blocked report retaining all
earlier attempts and raw responses.
The parent workflow must separately enforce the approved 8,000,000,000-byte new
retained runtime/model storage cap; this runner does not install or download a
runtime. No hosted transport exists in this pilot.

Replay imports raw response strings, so duplicate JSON keys and malformed or
oversized bodies reach the production parser. The export is regenerated and must
match exactly; altered prompts, source mappings, identity pins, hashes and changed
bridge binaries are rejected. Incomplete returned identity is blocked: a pinned
model rejection can fail at the production parser first; otherwise it is
`identity_unavailable`. A missing explicit expected identity is
`identity_unpinned`. Reported identity is not served-weight attestation, and an
artifact digest must never be invented from a requested alias.

For offline replay, `finished.py replay --freeze ... --responses ... --bridge ...
--output ...` accepts one response-list per exported fixture. Each response is
`{"window_index":0,"http_status":200,"body":"<raw JSON string>"}`. Prefix replay
is permitted for first-request compatibility inspection and is marked incomplete.

For descriptive raw component scoring, supply `--expected-answers` when freezing:
its JSON maps each fixture ID to one answer map per window, containing each
production choice question ID and an allowed expected label. This ground truth
is frozen but never enters model requests. `finished.py score --freeze ...
--responses ... --bridge ... --output ...` replays raw responses again and reports
each language separately through the existing classification metrics. Missing or
failed windows remain abstentions in the denominator. Scores are explicitly
uncalibrated; complete-item judgments, calibrated accepted-item precision,
resources and native acceptance remain unavailable, so this report cannot qualify
or adopt a model.

Reports retain all physical attempts and raw invented-only responses, while CLI
output contains metadata only. Exit status 2 denotes a recorded pilot blocker;
it is not an instruction to retry. CPU/RSS remain unavailable unless measured by
the separately scoped owned-runtime instrumentation. No compatibility result
qualifies or activates a model; full calibration/held-out, complete-item quality,
resource and native/source acceptance remain with the existing qualification owner.
