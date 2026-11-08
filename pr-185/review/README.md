# Review corrections

Matched before/after owned headless mock renders use the same invented partial-recovery and integrity-error states, fixture source and action sequence in EN/NL and light/dark. Only non-mutating test assertions are disabled in both screenshot snapshots so the known broken state can be captured. Full after interaction assertions run independently in a 24-image matrix, including minimum-size cases, with no failures.

Before source: c4a83a818e6827fdd4d30766010605216e716d8a. After source: 4b2f567416237fe82e7a048551549bd6914d5f78. The four original primary after images remain byte-identical at the corrected source. Source and image hashes are in manifest.json.

No real microphone, hosted/model inference, human or device qualification is represented by these mock images. The historical local Decider response remains blocked on missing serving revision/provider identity.
