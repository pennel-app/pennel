# Incremental prefix replay — 3 November 2026

This is an **owned synthetic fixture replay**, not model inference or product
qualification. All 12 candidate records remain unqualified; adopted records are empty.

- [Frozen context](freeze.json)
- Complete `report.json.gz` and `candidate-records.json.gz`
- [Offline cached-artifact verification](cached-artifacts.json)
- [Verification and acceptance receipt](../../../../docs/evidence/incremental-qualification-96.md)

The fresh run contains 1,440 finalized prefixes and 4,500 cases. Replay reproduces the
full report byte for byte. The report's semantic SHA-256 is
`1353118da9078e934fd700c00ffccb8d51f09aed63efcbcf68f66ec44bacb895`.
Compression uses deterministic gzip timestamps. Corpus and fixture predictions are
reproducible from versioned source:

```sh
python3 -B evaluations/decision-models/incremental.py --offline \
  --output build/incremental-96/offline
python3 -B evaluations/decision-models/incremental.py \
  --freeze build/incremental-96/offline/freeze.json \
  --predictions build/incremental-96/offline/predictions.jsonl \
  --output build/incremental-96/replay
```

Compare regenerated `report.json` with decompressed `report.json.gz`. No credential,
model load, network inference, artifact acquisition or paid evaluation is involved.
The authored ASR variant is not a natural-recording ASR measurement. Missing actual
inference, generated-claim and runtime evidence remains unavailable in the reports.
