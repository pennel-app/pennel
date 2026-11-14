# Peer transport qualification comparison

Owner: [#139](https://github.com/pennel-app/pennel/issues/139). This is an isolated
qualification experiment, not product replication or a Cloud migration.

## Boundaries

`peer-transport-qa` is absent from default Core features. Mobile preparation,
generation, Pods and build preflights require the matching explicit
`PENNEL_PEER_TRANSPORT_QA=1` graph. Production and TestFlight reject it. Interrupted
preparation publishes an unusable pending manifest before changing any products;
it cannot leave a QA library labelled by an older ordinary manifest.

The experiment opens explicit synthetic fixture paths only. It does not open a
meeting library, run recognition or inference, change providers, or modify the
existing sync service. Its transient relay has no offline file mailbox.

## Implementations

| Layer | Implementation |
| --- | --- |
| Shared file engine | `TransferManifest` and `PartialTransfer` recovered byte-for-byte from `22042c3`; SHA-256 `fb9833d26e6cb65e73fa93b573060c1c6ca6b8750ca85d60718304992edcacea` |
| Shared protocol | Bounded Rust `AsyncRead`/`AsyncWrite` wire, verified checkpoints, final application acknowledgement, cancellation and atomic no-clobber publication |
| Quinn | `0.11.12`, locked `quinn-proto 0.11.19` / `quinn-udp 0.5.16`, Tokio and Rustls/ring; private Apple fast datapath disabled |
| Native | One Swift Network.framework implementation for macOS/iOS; P-256 identities in Data Protection Keychain, TLS 1.3, exact peer certificate pins and reciprocal admission |
| Relay | Standalone TLS 1.3 byte-pair tool; expiring peer/run/candidate-bound tickets, pre-TLS admission, bounded forwarding, revocation, pause/resume and shutdown |

Direct means explicit LAN/reachable-address UDP for Quinn or TCP for native TLS.
Forced relay disables the direct path. Quinn then tunnels encrypted datagrams
through a bounded socket adapter; native TLS uses loopback ciphertext forwarding
with a one-shot OPEN/ACK coordinator. It is not a general stream multiplexer.
Both relay connections are outbound, and the relay does not terminate inner TLS.

Native byte-channel framing uses a four-byte length, at most 64 KiB of payload,
and a zero-length write-half EOF. Successful teardown drains gracefully; errors,
revocation and cancellation abort. These setup/framing/loopback costs belong in
the native candidate's measured overhead. Quinn over a reliable relay also has
nested congestion and head-of-line costs. Automatic WAN hole punching is not
implemented by either candidate in this experiment.

### Locked dependency review, 14 November 2026

Locked/offline Cargo metadata shows 32 packages added by the QA feature compared
with the lean graph, all from crates.io. Their declared licenses are MIT/Apache
variants, with one also offering Zlib. Shared dependencies still require their
own notices, including ring's Apache/ISC and BoringSSL notices, webpki's ISC and
Tokio's MIT. This is a declared-license and selected-advisory review, not a full
SBOM legal review or a general vulnerability guarantee.

The locked `quinn-proto 0.11.19` meets the patched ranges for the current
[stream-limit bypass](https://raw.githubusercontent.com/RustSec/advisory-db/main/crates/quinn-proto/RUSTSEC-2026-0523.md)
and [Retry-token panic/OOM](https://rustsec.org/advisories/RUSTSEC-2026-0524.html).
Locked `quinn-udp 0.5.16` includes the
[Apple truncated-datagram spin fix](https://rustsec.org/advisories/RUSTSEC-2026-0538.html),
which matters with the private datapath disabled. The review also checked
Rustls 0.23.46, webpki 0.103.16, Tokio 1.54.1, ring 0.17.14, tokio-rustls 0.26.6,
time 0.3.55 and both selected SHA-2 versions against their published ranges.
The exact sources and ranges are retained in the dependency receipt below.

Quinn selects Tokio and Rustls/ring. No private Apple datapath, AWS-LC, OpenSSL,
netdev, netwatch, Git forks or alternate registries are in the new selected
graph. The wider Core Rustls feature union includes TLS 1.2; the QA connection
builders explicitly permit TLS 1.3 only. Native TLS protocol updates are tied
to the supported Apple OS, while its application framing/bridge remains owned
here.

## Limits and evidence interpretation

- Quinn configures 16 pending incoming attempts, 64 KiB per attempt and 256 KiB
  aggregate pending buffering before endpoint creation. A real Initial-packet
  saturation regression verifies admission and capacity recovery. Locked
  `quinn-proto 0.11.19` drops saturated Initials; the client does not necessarily
  receive the refusal described by the API documentation.
- Native bounds delivered connections, read/write queues, frames and handshake
  deadlines. Apple does not document a total pre-admission kernel/TLS allocation
  ceiling. Application limits are not a claim about total process memory.
- Quinn uses 2 MiB stream / 4 MiB connection receive windows, a 4 MiB send window,
  two bidirectional streams, no unidirectional streams and a 30-second idle
  deadline. The UDP relay adapter holds at most 32 × 1,200-byte packets per
  direction. The Rust native bridge is a 64 KiB duplex channel with two runtime
  workers and eight blocking workers. Native admits 16 delivered channels,
  bounds controls at 16 KiB, and applies 10-second handshake / 30-second step
  deadlines to admitted work.
- The relay has 16 setup permits acquired before TLS, at most two paired clients
  for this run, 16 KiB headers and 64 KiB forwarding buffers per direction.
  Its tests exercise setup-permit refusal/recovery, slow and paused forwarding,
  expiry, revocation and shutdown. Those are application-level bounds.
- The shared engine caps manifests at 2 MiB and blocks at 1 MiB. Host trials use
  128/256/1,024 KiB blocks, three repetitions and the retained 8,388,633-byte
  fixture, plus empty, one-byte and small-note fixtures.
- Content bytes, opaque relay-forwarded bytes and complete network traffic are
  different counters. An unavailable full-network count is unknown, never zero.
- Host CPU/RSS sampling covers owned process IDs at 100 ms intervals. Phone
  sampling covers the app process through public Mach/getrusage counters at
  50 ms intervals. Neither includes unrelated services or extensions.
- Capture trials measure native microphone start/release and delivered sample
  counts while transferring or deliberately stalled. They do not establish ASR,
  a real call, speaker attribution, permanent background execution or battery
  acceptance. Small timing samples use median/max, not a p95 claim.

## Verification ledger

The implementation and source review passed the following checks before the
final physical campaign:

| Check | Result |
| --- | --- |
| Core default / lean | 1,382 / 1,365 passed; 9 / 6 explicit ignores |
| QA Core / retained file regressions | 13 / 14 passed; strict Clippy and formatting passed |
| Actual enabled ARM64 iPhoneOS / simulator compile | Passed without netdev/netwatch patches |
| Desktop frontend / backend | 538 / 426 passed; frontend build and backend Clippy passed |
| Existing Mac platform | 241 passed |
| Mobile UI / widget | 166 / 7 passed; typecheck passed |
| Mobile generation plugins | 22 passed, including explicit pending-state rejection in generation/Pods and ordinary Preview cleanup |
| Mobile artifact preparation regressions | 15 passed, including both failed graph-switch directions |
| Controller | 29 passed, including evidence, capability, capture, metric and identity-reuse validation |
| Relay | 19 checks passed; strict Clippy and formatting passed |
| Native signed Mac | 7 unskipped regressions plus persistent-listener reversal smoke passed |
| Hosted QA simulator | 147 runtime + 36 keyboard passed; optional phone/capture cases disabled |

An initial unconstrained parallel backend run timed out an existing mock fixture;
the isolated test and full 426-test run with four test threads passed unchanged.
Retained intermediate failures are not qualification approvals.

The final safety pass additionally rejects truncated magic/length/manifest and
block input, malformed manifests and oversized allocation requests. A truncated
second block leaves only the first durable receipt. Actual inode/content source
replacement is rejected under the old manifest; filesystem write obstructions
and a competing destination publication preserve unrelated data and verified
blocks without sending a completion acknowledgement. These exercise real file
errors, not an injected ENOSPC/fsync failure. A stalled shared-wire read cancelled
in 0.380 ms in one host component test, with an explicit 250 ms deadline; this is
not a physical-device or all-phases cancellation measurement.
An additional actual mutually authenticated Quinn channel test observed a
pending peer read, revoked the peer with the source stream still retained, and
measured read failure in 0.255 ms under a 250 ms deadline. Later local dials and
remote application access were rejected. Native signed regressions separately
cover active read revocation and buffered-byte rejection.

One additional native host campaign held 64 plain TCP clients without completing
TLS. All 64 connected; 16 sockets closed during the 8–12 second observation band,
and 48 remained TCP-open after 14 seconds. Those remaining sockets can be in OS
or listener backlog state, so the admitted handshake watchdog is not a global
socket lifetime guarantee. The target helper's baseline median RSS was 16.81 MiB
and stalled-phase peak was 18.78 MiB; 140 samples observed 0.02 seconds of CPU.
After all owned sockets were closed, a newly approved 1,048,615-byte transfer
passed exact-hash verification. This finite campaign establishes observed
recovery and memory only; it does not establish an OS allocation ceiling or a
sustained denial-of-service bound.

Both final transport sources passed 78 direct and 80 forced-relay host cases,
including both-direction restart, corrupt checkpoint and revocation. Abrupt
mid-block recipient death was exercised on the paced forced-relay path only.
Those initial functional runs overlapped native
builds and must not be used as a controlled performance comparison.

The physical campaign is recorded separately below after validating actual
coordinator receipts. A successful Xcode command with zero selected tests is not
acceptance; the controller requires a real returned file and exact SHA-256.

### Physical path correction

The first physical calls reached identity, transport and fixture readiness, then
failed before the first durable block. A path-free syscall probe found that
Foundation retained the `/var` symlink alias in its resolved temporary URL.
Directory open/sync and same-directory hard links passed. The caller now applies
public POSIX `realpath` only to its freshly created private QA directory. The
retained Rust engine and its symlink refusal are unchanged.

### Final comparison

The final matched host campaign passed all **316 cases**: 78 direct and 80
forced-relay cases per candidate. No owned native builds or phone transfers ran
during these timings. Hardware was an Apple M4 Pro Mac with 48 GiB RAM. Each
binary timing cell contains six measurements: three repetitions in each direction.
Both endpoints and the relay run on that host with loopback addresses; these
host timings do not include a physical Wi-Fi or WAN path.
Elapsed time covers the receive request through verified durable acknowledgement.
The same signed helper and bundled Core contain both selectors; these compare
the selected runtime path in a shared QA host, not separately linked install
sizes.

| Host route | 128 KiB median / max, s | 256 KiB median / max, s | 1 MiB median / max, s | Cumulative endpoint RSS peak by 1 MiB phase, MiB |
| --- | ---: | ---: | ---: | ---: |
| Quinn direct | 0.750 / 0.798 | 0.429 / 0.454 | 0.207 / 0.213 | 28.86 |
| Quinn forced relay | 0.816 / 0.834 | 0.482 / 0.494 | 0.237 / 0.251 | 28.44 |
| Native direct | 0.830 / 0.859 | 0.491 / 0.502 | 0.254 / 0.267 | 31.03 |
| Native forced relay | 0.838 / 0.861 | 0.508 / 0.524 | 0.266 / 0.286 | 32.80 |

Each route reuses its endpoint processes across 128 → 256 → 1,024 KiB trials.
The RSS column is the process-lifetime peak reached by that final phase, not an
isolated block-size memory measurement.

The earlier matched run exposed common harness memory accumulation: the
long-lived Swift command loop had no explicit Foundation autorelease drain.
Pools now surround command reads, execution/output and native callbacks. Later
endpoint peaks fell from roughly 360–380 MiB to 29–33 MiB. Both runs are retained;
the earlier RSS must not be attributed to either transport. This bounded campaign
does not establish a universal leak-free guarantee.

These results support retaining the existing 1 MiB fixture block default. They
do not establish that size as optimal for cellular, large production libraries,
disk pressure or every supported device.

The final sampled CPU totals below are cumulative lifetimes of the two endpoint
processes at the end of the 72 timed fixture operations on each route. They
include startup, verification and unchanged repeats. They are not per-transfer
CPU measurements. Relay CPU belongs to a separate process.

| Host route | Endpoint A / B CPU, seconds | Relay CPU, seconds |
| --- | ---: | ---: |
| Quinn direct | 2.65 / 2.62 | Not used |
| Quinn forced relay | 2.93 / 2.87 | 1.44 |
| Native direct | 2.82 / 2.82 | Not used |
| Native forced relay | 3.64 / 3.64 | 0.35 |

All four personally signed optimized phone routes passed again on the final
preserved QA app/test bundle: first checkpoint
1,048,576 bytes, resumed 7,340,057 bytes, unchanged repeat zero content bytes, and
8,388,633 returned bytes independently checked by the Mac. The source, received
and returned fixture hashes agree. The native transport source is
`fdd156d4cf4ca8d857e431fe7402bd5c7b268fa2a73d0721ae654d59ba9a0952`.

The phone is an iPhone 16 Pro running iOS 27.2; the SDK is 27.2, Xcode is 27.2 and
Swift is 6.4.2. The test bundle uses optimized Release Swift with testability,
single-file compilation and retained test symbols. The verified personal Apple
Development certificate signed the app. These are controlled native tests, not
store or general meeting-workflow acceptance.

| Phone route | Complete checkpoint/repeat/return campaign, seconds | App CPU delta, seconds | Sampled peak app RSS, MiB |
| --- | ---: | ---: | ---: |
| Quinn direct | 3.331 | 0.989 | 175.9 |
| Quinn forced relay | 3.754 | 1.334 | 176.1 |
| Native direct | 5.509 | 1.449 | 213.8 |
| Native forced relay | 3.882 | 1.068 | 199.8 |

Each row is one campaign, including setup, local verification and both transfer
directions. The app process also contains Expo, React Native and the common
Core. The final run executes native direct first and includes UI/runtime warm-up.
These are descriptive
observations, not isolated library footprints or statistically established
latency advantages.

The earlier complete capture campaign passed all 60 real microphone cycles.
Startup now includes
the first nonempty PCM callback, with a two-second readiness deadline. An initial
short baseline failed with no transport active; its precise failing subcondition
was not logged. Every passing cycle delivered at least 6,256 samples and reported
no raw callback in the 20 ms observation window after stop.

| Candidate / phase | Start median / max, ms | Stop median / max, ms |
| --- | ---: | ---: |
| Quinn baseline | 279.13 / 337.53 | 165.18 / 177.60 |
| Quinn active | 280.95 / 329.47 | 164.08 / 170.79 |
| Quinn paused relay | 284.90 / 332.46 | 172.61 / 257.17 |
| Native baseline | 286.15 / 340.67 | 172.26 / 185.82 |
| Native active | 281.30 / 310.96 | 129.95 / 178.92 |
| Native paused relay | 284.74 / 330.44 | 171.69 / 194.08 |

The load is a 256 MiB synthetic transfer on the forced-relay path. The pause is
applied only after the file manifest is received, so the stalled test exercises
an established transfer instead of a failed handshake. All ten operations finish
while that transfer remains pending; no network watchdog release is needed.
This measures the raw microphone component, not full meeting-archive
finalization. Sample counts cannot prove zero physical audio-frame drops.

A repeat on the final preserved app/test bundle failed after 30 successful Quinn
cycles, when entering a native-candidate no-transfer baseline: capture reported
`running=false`, zero samples and no callback after stop. The real Swift case
failed after 45.925 seconds. There is no recorded interruption/route diagnostic
establishing the cause, so this cannot be attributed to a transport. The failed
run and all completed phase receipts are retained. A fresh-process retry does
not erase this missing-audio failure or establish recording reliability. That
one bounded retry also failed, in the Quinn no-transfer baseline before any
transport started: `running=false`, zero samples, no callback after stop, after
3.111 seconds and with no completed phase receipt. Further broad retries stopped.
The earlier 60-cycle table is historical passing evidence, not acceptance of the
final app. Both final missing-audio failures leave the recording gate unresolved.

### Lifecycle checkpoint and reload

The signed checkpoint phase passed one real Swift test in 7.647 seconds. Both
candidates durably stored the first 1,048,576 bytes of the exact retained
8,388,633-byte fixture, with no completed target exposed. The separate resume
phase retains both device identities and requests only the remaining 7,340,057
bytes. Its reload witness requires a newly loaded React surface presenter that
matches the current host, excluding a queued initial-load notification.

The owner confirmed lock for five seconds, unlock, Home and app return, Wi-Fi off
for five seconds, Wi-Fi on and return to the Mac's Wi-Fi. The separate signed
resume test passed one real Swift case in 27.281 seconds. It observed the warm-up
replacement, then another newly loaded React presenter while both transfers
remained active. Both independently resumed exactly 7,340,057 bytes, repeated
with zero content bytes and returned 8,388,633 bytes verified by the Mac against
the retained SHA-256. The QA app and test executable hashes below stayed
unchanged throughout these two phases.

These are operator-confirmed checkpoint-survival transitions followed by real
native transfer/reload acceptance. They do not prove that iOS actually suspended
the process, transferred while locked, or delivered via cellular WAN. The
coordinator remained on the private LAN. The concurrent lifecycle sampler covers
both candidates in one app and is not a per-candidate performance comparison.

### Ordinary restoration and review validation

The default-feature Core Release build and ordinary mobile Release build passed
after QA. Clean/repeated feature-off generation, locked Pods and seven actual
ownership checks passed. The final manifest matches Core input SHA-256
`ec1fcffb922faabdc152f6fedd45f43c830f823fe94a8913b876798dcbf37556`,
declares `features: []` and both iOS targets, and its bindings contain no QA
comparison types. Explicit pending-state checks also reject a `preparing: true`
manifest even if it contains an ordinary feature list; the added validation
does not change the evaluated pod attributes or lockfile.

The personally signed ordinary Preview app was verified to have the normal
Keychain namespace, no QA profile marker and no runtime-test plugin, then
installed in place without uninstalling. The final host Core product was
restored to default features after mobile bindgen preparation. All owned QA
coordinators and test commands stopped, and the task-owned simulator is shut
down. No Argent runner or Metro session was started; no post-install phone
inspection or launch restarted a runner. Signed QA products, symbols and failed
evidence remain retained separately.

Normal ordinary-build validation is recorded on [PR #266's checks](https://github.com/pennel-app/pennel/pull/266/checks)
for its exact current head. Its CI status is separate from the enabled QA
physical campaign. Passing ordinary CI cannot clear the missing-audio gate.

### Decision and remaining gates

Neither candidate is approved for production adoption by this experiment.
The observed missing-audio baseline failure leaves the recording gate unresolved.
Complete network-byte accounting, sustained adversarial flood measurements,
abrupt mid-block process death on the direct route, and physical
cellular/background transfer acceptance remain open. A durable
checkpoint surviving a network change is distinct from a transfer completing
over cellular or while suspended. The full meeting recording and archive
workflow also needs acceptance beyond the raw microphone component measured
here. Issue #139 stays open; Cloud migration stays gated.

The measured host file workload favors Quinn on elapsed time and sampled endpoint
memory, with explicit pending-connection controls. A future native-only backend
could omit a bundled QUIC implementation, but this experiment still needs Swift
identity handling, framed callback bridging, loopback pumps
and relay role coordination. The native implementation covers Apple platforms;
Windows/Linux would need another backend and parity acceptance. Quinn uses the
shared Rust path, but this campaign does not establish non-Apple qualification.
Quinn also needs a custom bounded UDP tunnel for relay use. Both require
identity lifecycle, reachability, relay operation and mobile
reconnection work before product sync. Those maintenance costs are unresolved;
these results do not select a transport automatically.

### Revisions and retained evidence

The experiment started from App revision
`69967894e86c874d826edd17df7bec57b7495e8a`. Its initial implementation is
`7f19bd236e9491cc5b00959108f5508bb3660502`; the private phone capability-file
correction is `50680b442955cdd2fa884a344b5630fe6cfa40a3`. Subsequent physical path,
autorelease-pool and harness corrections are
`299bbbbee2b839cb7a757314ae9181e2028e4bba`. Final safety and lifecycle tests are
`c3ea214890c0bfaa3e2a49c425c292664580644e` and
`941df3896938104a3d290c566a37d101adfe8da0`, respectively. The additional active
QUIC revocation regression is `217c0e2f9c74a4571f6df4601252ca24a3c3896f`.
The host
receipt records the earlier HEAD, so the source hashes below additionally bind
its then-uncommitted measurement code. Later changes to test-only coverage do
not change the transport under measurement.

| Artifact or source | SHA-256 |
| --- | --- |
| Matched host receipt, logical ID `peer-comparison-host-pooled/result.json` | `a07a44c653f16be0eabec279fbca52838596e8e3e288a1898cb37769b4207162` |
| Targeted dependency review receipt | `34beda4cf6b7304f2fa5a5c7c969e4162d845ef6d4ffa56a79892cfc6449d9e9` |
| Native stalled-handshake receipt | `10f04672d7b30894e5bfb265678fd996d6febb93684b3dc835b349f8fa5bd60e` |
| Shared signed host executable, both selectors | `877f5111c8e8b795a8989289351f6846817817a8ddd4c590b711b16d3e0ebf73` |
| Host bundled Core dylib | `2cc440ed373af2b98b9d06779babaf49921dc27d73f7b64bc65af3a0c2f2808e` |
| Relay executable | `3f534eebf4fdf5cc6244b131981a091bd07c415ea216be500cd423f26dbb25ae` |
| Python controller | `f0f8d3ac8f33dcd5f305583e35d889227e81e3abb19c0eb65cf13e9a7eac9584` |
| Swift command loop | `1313aa23ba25a07e7e88d1cb3b119ecef3aea91ee6f0b1442462d2296b8b96d2` |
| Native callback/identity bridge | `8cb2e8f73c9a97c2d745813d0b27f0b611b1c2e9e33eecb58efdb2431d754418` |
| Final lifecycle test source / immutable QA snapshot ID | `746e9ca68efef2adfa95fbf1d87dcf94422008e6efc7db9d8e9b7d2602f31dd3` |
| Final preserved QA phone app executable | `2419dc2c2b6e8529ecc3a5ce9fad74b2935fed86bb79afa20eab292aaa80dab3` |
| Final preserved QA runtime test executable | `d62ff6d21d93656eca81ecbf6d87dafeeb6ef264b25eb1c2d0bc2bcbdaad6722` |
| Restored ordinary Preview app executable | `4fff55de7b5c37805b472817d617bacbb1c57bea9182b20c159cfe5dc2a08897` |
| Restored default-feature host Core dylib | `4d805da5411a04144742cfa48a98a755982aa3f4590fd284e778244262343ae3` |
| Core lockfile | `64c12fc2632c6e73591a3bf7f6cb0748ac232b12d0bcbd45890eecbc4599a1b5` |
| Desktop backend lockfile | `fd23d75a8bec1205090a1bf475883219073b641a0ebb2be35943a602e99344bf` |
| Relay lockfile | `efbc563197733e789d05c707d371fb49f027a6aa2cf0a3c759a043496422cff3` |

Private run configurations, tickets, raw runner logs and result bundles remain
in ignored QA storage. They are not uploaded as public artifacts. Physical
receipts use logical IDs `phone-final-roundtrip` and
`phone-final-roundtrip-controllers` for the final phone table. Earlier passing
`phone-roundtrip-canonical`, `phone-capture-r2` and
`phone-roundtrip-controllers-r3`, plus failed and zero-test attempts, remain
separate. Final phone repeats use the preserved app/test executable hashes above.
The public tables contain only synthetic fixture counts, timings,
hashes and process metrics. Host and physical evidence must remain separate.

## Reproduction

Start in the App checkout, with the repository's existing toolchain and storage
guard. Do not globally change Cargo targets or host signing settings.
If the checkout links an existing local toolchain, resolve its real directories
for `CARGO_HOME` and `RUSTUP_HOME`; the symlink aliases caused an Askama/UniFFI
derive path failure during this run. The validated tools were Rust/Cargo 1.99.1,
Node 22.22.3, npm 10.9.8, Python 3.15.2 and CocoaPods 1.16.2.

```sh
scripts/with-toolchain.sh cargo test --manifest-path core/Cargo.toml \
  --no-default-features --features peer-transport-qa --lib --test peer_compare_transfer
scripts/with-toolchain.sh cargo clippy --manifest-path core/Cargo.toml \
  --no-default-features --features peer-transport-qa --lib --tests -- -D warnings
worktree-storage run -- bash scripts/build-peer-comparison.sh
```

Supply the verified personal `APPLE_SIGNING_IDENTITY` and existing
`PENNEL_MAC_PREVIEW_PROVISIONING_PROFILE`, then run
`bash scripts/sign-peer-comparison.sh`. It verifies the profile/team/certificate,
bundles the exact core library, rewrites its load path and signs the owned helper.
Transport identities are separate from Apple signing identities.

```sh
python3 -B scripts/peer-transport-comparison.py host \
  --quinn-binary 'build/peer-comparison/Pennel Peer Comparison.app/Contents/MacOS/peer-comparison-probe' \
  --native-binary 'build/peer-comparison/Pennel Peer Comparison.app/Contents/MacOS/peer-comparison-probe' \
  --relay-binary tools/peer-comparison-relay/target/release/pennel-peer-comparison-relay \
  --output build/peer-comparison-new-run --run-id comparison --mid-block-crash
```

Output directories must be new and Git-ignored. Private configurations, tickets,
identities and diagnostics stay there. Use only the allowlisted public summary
for publication. Native unsigned SwiftPM tests cannot establish Data Protection
Keychain acceptance; the signed test runner uses the existing personal Preview
profile and the public SDK Swift Testing runner entrypoint.

For phone preparation keep `PENNEL_PEER_TRANSPORT_QA=1` through Core preparation,
Prebuild and locked Pods. The optimized test build requires testability and
single-file Swift compilation; whole-module Release eliminated three unrelated
existing test-only extension symbols. Sign only with the verified personal team
and certificate. Pass coordinator capabilities through the fixed files
`Documents/QA/peer-transport/qa-roundtrip.json`, `qa-capture.json` and
`qa-lifecycle.json`; runner environment flags select them without containing
their secrets. Lifecycle coordinators use fresh output directories and
`device --identity-file` pointing at the prior owned private QA metadata file,
so controller restart retains the same Mac Keychain identity without copying
private keys.

The signed build used the following additional options. `$PHONE_UDID` is the
explicit selected phone and `$PERSONAL_DEVELOPMENT_CERT_SHA1` is a verified
personal Apple Development certificate from team `B538Q2K76C`; neither variable
selects an automatic fallback identity.

```sh
worktree-storage run -- env PENNEL_VARIANT=preview PENNEL_PEER_TRANSPORT_QA=1 \
  make mobile-prebuild
worktree-storage run -- env PENNEL_VARIANT=preview PENNEL_PEER_TRANSPORT_QA=1 \
  bash scripts/ci-xcodebuild.sh build-for-testing \
  -workspace apps/mobile/ios/Pennel.xcworkspace -scheme Pennel \
  -configuration Release -destination "platform=iOS,id=$PHONE_UDID" \
  -derivedDataPath apps/mobile/build/DeviceTestDerivedData \
  -parallel-testing-enabled NO -allowProvisioningUpdates \
  ARCHS=arm64 ONLY_ACTIVE_ARCH=YES ENABLE_TESTABILITY=YES \
  SWIFT_COMPILATION_MODE=singlefile DEAD_CODE_STRIPPING=NO \
  COPY_PHASE_STRIP=NO STRIP_INSTALLED_PRODUCT=NO \
  CODE_SIGNING_ALLOWED=YES CODE_SIGN_STYLE=Automatic \
  DEVELOPMENT_TEAM=B538Q2K76C CODE_SIGN_IDENTITY='Apple Development' \
  EXPANDED_CODE_SIGN_IDENTITY="$PERSONAL_DEVELOPMENT_CERT_SHA1"
```

The native simulator Release and unsigned device Release builds used the same
enabled graph. Clean and repeated non-clean Prebuild plus locked
`pod _1.16.2_ install --deployment` passed; QA flags belong only to Pennel's
app/runtime-test/native-module targets, not SDK pods. Ordinary graph restoration
repeats preparation with `PENNEL_PEER_TRANSPORT_QA=0` before building.

Select exact Swift methods with Xcode's full `-only-testing:Target/Suite/method()`
argument. Suite-only `OnlyTestIdentifiers` in a copied run file produced zero
tests and was rejected as evidence. Restore ordinary Preview without uninstalling
after qualification, and stop only the owned phone/simulator sessions.

| Physical test | Exact selector suffix after `PennelRuntimeTests/` | File-selection flag |
| --- | --- | --- |
| Four route round trips | `PeerComparisonQualificationTests/syntheticMacIPhoneComparison()` | `PENNEL_PEER_COMPARISON_CONFIG_FILE=1` |
| Sixty microphone cycles | `PeerComparisonCaptureTests/microphoneCaptureDuringActiveAndStalledTransfers()` | `PENNEL_PEER_CAPTURE_CONFIG_FILE=1` |
| Checkpoint and reload/resume, separate phases | `PeerComparisonLifecycleTests/checkpointAndResumeAcrossJavaScriptReload()` | `PENNEL_PEER_LIFECYCLE_CONFIG_FILE=1` |
