# Device connection availability and recovery

Tracked under [#77](https://github.com/pennel-app/pennel/issues/77), with the repair in [PR #86](https://github.com/pennel-app/pennel/pull/86).

Desktop and iPhone check the configured service before creating or joining a connection. Unsupported or disabled services keep controls visible with an explanation, **Check again**, and email/recovery alternatives. Incompatible services ask for an update. Access denial and temporary network/server failures have separate localized guidance. The interface presents one connection failure rather than repeating native, action and phase errors.

## Shared interface

`PennelEngine.cloud_link_availability()` returns `CloudLinkAvailability { available, reason_code }` through UniFFI, desktop `cloud_link_availability`, and mobile `cloud.linkAvailability`. This unauthenticated read does not change credentials, the current connection, or a pending provisioning journal. Missing/disabled support returns `pairing_unavailable`; an incompatible response returns `pairing_incompatible`. Transport and service failures can be checked again. Raw upstream response bodies are not interface copy.

The core checks admission again inside start/join, including connection and account ownership after the network check. Disabling new admission does not prevent an existing connection from polling, being approved, completing provisioning, or being cancelled. Failed admission retains its role and error code. Restored mobile views are checked against the current account even on a fresh screen visit.

## Recovery boundaries

Provisioning validates transferred key/grant material before writing a journal. Temporary proof/poll failures retain pending recovery evidence. A proof-only check does not rotate a single-use refresh token. If the server has purged completed-request metadata and retained proof has expired before acknowledgement was recorded locally, use fresh pairing or email/recovery. A previously acknowledged device may need ordinary removal from the trusted device list to free a slot.

Independent engineering review and automated tests are distinct from a formal cryptographic audit, installed-device acceptance and deployment acceptance.

## Isolated signed iPhone QA

An optional `PennelQAProfile` marker is accepted only in `app.pennel.ios.preview`, with a 1–64 character ASCII alphanumeric/hyphen profile and exact `PennelKeychainService` value `app.pennel.ios.preview.qa.<profile>`. Invalid or mismatched configuration fails before opening normal stores. The marker is injected only into the copied QA product before signing; ordinary app configuration does not set it.

The marked product separates data, meetings, preferences, credentials and handoff storage. It skips shared keyboard/widget publication, stale activity/result cleanup and background registrations. Restore the normal signed Preview product after the run. Use the existing personal signing team only.

## Verification receipt — 31 October 2026

- Release core, 714 core tests (seven existing ignored), strict Clippy and formatting passed.
- Desktop frontend build and 233 tests, 224 backend tests, and English/Dutch light/dark interaction checks passed. The migration fixture uses its temporary library rather than the user's open canonical library.
- Mobile typechecking, 99 UI tests, 10 plugin, seven widget and 76 Swift helper tests passed. Clean/repeated Prebuild and six pod/source ownership checks passed.
- Three deployed staging core/server pairing paths and encrypted synthetic meeting/note synchronization passed. These are separate from camera-based and signed physical-device acceptance.
- Simulator and unsigned iPhoneOS Release builds passed. Personally signed Mac/iPhone candidates passed both QR roles, manual entry, explicit approval with matching verification codes and encrypted first sync on staging. The phone reported zero uploads and two synthetic files downloaded. Hosted native tests passed: 104 app/runtime and six keyboard tests.
- Production Cloud 1.0.7 was promoted with admission disabled, verified, then enabled. Three production core/server pairing paths and synthetic encrypted sync passed. The signed Mac and physical iPhone then completed an additional production connection with matching verification codes, explicit approval and both synthetic Markdown files downloaded.
- Both QA sessions were signed out and the normal personally signed Preview product was restored without uninstalling. All 45 original iPhone Preview Documents entries retained identical metadata after the full staging/production run and restoration. This is metadata preservation evidence, rather than a full content-hash comparison. Production and staging phone QA use different isolated profiles.

Integration with the later keyboard/TestFlight changes preserves both sets of behavior. The new keyboard writing server is excluded from marked QA bootstrap, and typing/feedback preferences remain scoped to the QA suite. A native regression covers the injected typing preference/reset/mirror paths without changing the normal keyboard store. Required CI is rerun on that combined source before merge.

The physical candidate used macOS 27.0.1 on Apple M4 Pro and iPhone 16 Pro/iOS 27.0.1, with the existing personal team `B538Q2K76C`. This does not establish notarized distribution, signed production upgrades, keyboard cold start, broader accessibility or App Review acceptance. Those gates remain in their owning tasks.

Exact local logs and owned renders are retained under ignored `build/device-link-activation/` and `build/device-connection-activation/`. The private Cloud repository retains protocol review and operational receipts.

Owned synthetic renders: [single service error, English/dark](evidence/device-connection-2026-10-31/desktop-service-error-en-dark.png) and [unavailable admission, Dutch/light](evidence/device-connection-2026-10-31/desktop-unavailable-nl-light.png).
