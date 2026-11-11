# Pennel v0.1.45 published — 11 November 2026

The owner explicitly authorized publishing the merged Mac redesign and updating Homebrew. The previous publication hold was superseded. Existing automatic Mac workflow43028588823, attempt2, completedSUCCESS on exact merged source `27b75bec05bd78370dab4d8f277bbfae25b06367`. Published at 2026-11-11T15:20:47+01:00 (Europe/Brussels).

- Stable latest release: https://github.com/pennel-app/pennel/releases/tag/v0.1.45 (non-draft, non-prerelease).
- Tagv0.1.45 resolves to merged main27b75bec, complete reviewed treec9e95177 unchanged. All27 redesignPRs remain merged.
- Exactly two assets: Pennel_0.1.45_aarch64.dmg (15733236 bytes) and SHA256SUMS.txt. Downloaded the actual published DMG and checked its SHA-256 against both the checksum asset and GitHub's asset digest.
- SHA-256: `c7dc91aaa000d4a4e31812c1e4bf638105d984cc03ed9917c2920876599a4f1c`.
- Read-only mounted the owned downloaded DMG, verified bundled version/build0.1.45, arm64, minimum macOS14.4, expected permission descriptions, menu-bar identity and deep/strict code signature. Its designated requirement retains the pinned existing release certificate. The app was not launched or installed; the disk image was detached successfully.
- Homebrew main `ade2e4fa48e32e8bbd97eadb4f36fbced5ac8a4d` now contains version0.1.45, the exact released DMG URL and matching SHA-256; Ruby syntax check passed.
- Actual GitHub statusCheckRollup on27b75bec is SUCCESS: 16 evaluated checks, no failed/pending contexts. Main core/evaluation/mobile and the automatic Mac release run completed successfully. Earlier cancelled attempts remain in history; no status was falsified or deleted.

The release uses the existing self-signed release identity, not DeveloperID/notarization. Existing signing/upgrade/device/VoiceOver/focus/provider/qualification/store acceptance remains separately owned; this static artifact inspection does not establish it. Windows/Linux product builds remain skipped. No credentials, signing identity, routine platform policy, source code, tracked version file, or user installation was changed for publication.

Machine proof: mac-release-public-verification.json. Workflow: https://github.com/pennel-app/pennel/actions/runs/43028588823. No active publication/download/mount task remains.
