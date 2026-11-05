# Lightweight local worktrees

The `Pennel lightweight` local environment performs no automatic dependency
installation or build. Reuse a suitable existing checkout for follow-up work;
use an isolated worktree when concurrent changes need it.

Install only the component you are working on, using the explicit desktop or
mobile dependency action. Each checkout keeps its own writable dependencies;
do not symlink one `node_modules` between branches. The install actions use the
existing lockfiles and package manager.

`make test` runs the ordinary Rust test profile without first building the
release library. `make core` retains the explicit release-library build.
`make verify-core` builds that release library and then runs the tests, stopping
if either step fails. Release, FFI and packaging verification remain required
where the change affects them.

Desktop frontend tests and mobile JavaScript checks are separate explicit
actions. They require their component's dependencies. Swift/native tests,
clean and repeated Prebuild, device builds and release acceptance still use
the existing documented flows; the JavaScript actions do not replace them.

The macOS iOS build action is explicit and invokes `worktree-storage run` when
the personal guard is available on PATH or at `$HOME/.local/bin`. Other Macs
use the existing `make mobile-ios` route. Do not run native builds merely to
prepare a new worktree. Keep compiler intermediates isolated and preserve
release products, debug symbols, native build trees and verification evidence.

There is no automatic cleanup hook. Whole-worktree deletion needs its own
retention checks for ignored artifacts; a Git snapshot alone does not retain
all release and symbol data.

Codex discovers `.codex/environments/environment.toml` as a preferred local
environment, but a saved workspace choice, including None, can override it.
Select `Pennel lightweight` in Codex when needed. Saving this file does not
verify the selection of an existing chat. See the official
[local environments documentation](https://learn.chatgpt.com/docs/environments/local-environment).

## Hoppscotch development tools

This checkout includes project-scoped Codex and Claude Code configuration for
the official Hoppscotch MCP. Follow the [workspace maintenance guide](../docs/agents/hoppscotch.md)
for sign-in, allowed metadata tools, secret handling and the CLI fallback.
Restart the host to discover the server; no global registration is needed.
