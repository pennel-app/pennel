# Mock writing-profile UI evidence

All images use synthetic fixture data in the owned headless desktop frontend. No real account, meeting, recording or model was accessed. They establish frontend behavior only.

`before/` loads desktop frontend source from `origin/main` at 9c1227bba9651a112e14b1f4a27a9f71d3d6691d. `after/` uses this branch. Both use the exact same fixture script and per-case input, verified by the hashes in their manifests.

Ready/off production and existing Preview cases cover English and Dutch in light and dark. `interactions/` passes production first-use consent, production ready-profile controls, and existing Preview controls in all four locale/theme combinations. This covers independent sharing, production connector copying, corrections/focus, exclusion invalidation, pause retention and confirmed deletion.
