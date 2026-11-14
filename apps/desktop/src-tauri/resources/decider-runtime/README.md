# Managed local Decider runtime

The desktop before-bundle hook prepares the pinned native runtime in a fresh
packaging copy. It signs that copy with the app's selected identity, refreshes
its distribution manifest after signing, and copies the resulting bytes here.
The source build is never signed or changed in place.

Generated libraries, manifests and dependency license notices are ignored in
source control. Apple Silicon macOS is the supported runtime target. Other
targets retain this placeholder and report local Review as unavailable.

Model weights are acquired separately through an explicit in-app download.
Neither this placeholder nor packaging establishes model qualification.
