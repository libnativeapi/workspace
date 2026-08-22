# libnativeapi workspace

This is a workspace repo: `core/` and `bindings/*` are git submodules of independent repositories under the `libnativeapi` GitHub org, while `tools/codegen` and the `./codegen` script live directly in this repo. Work inside a submodule is committed and pushed from that subdirectory; the workspace repo tracks submodule pointers, the code generator, and this shared tooling.

## Architecture

- `core` — the C++ core library (repo: `nativeapi`). The source of truth for the native API surface (windows, tray icons, menus, displays, keyboard, dialogs, storage, etc.) with per-platform implementations (macOS/Windows/Linux).
- `tools/codegen` — in-repo (not a submodule) Rust workspace that generates the C ABI and language bindings from the core headers. Three crates: `shared` (libclang parser, IR, naming), `capi` (C ABI + umbrella header), `bindings` (Rust/Swift/Dart, consumes the IR JSON emitted by `capi`). Run via `./codegen` at the workspace root; `./codegen check` verifies generated files are up to date.
- `bindings/nativeapi-flutter`, `bindings/nativeapi-rust`, `bindings/nativeapi-swift`, `bindings/nativeapi-kotlin` — language bindings wrapping the core library. The Rust binding layers `crates/nativeapi` (safe API) over `crates/cnativeapi` (FFI).

A change to the core C++ API typically ripples: edit headers in `core`, run `./codegen`, then fix up each binding (see tools/codegen/README.md for the required manual follow-ups). When making such a change, update all affected repos in the same session and verify each still builds.

## Conventions

- Each submodule tracks `branch = main`. Use `make sync` to fast-forward all of them; `make status` to see dirty state everywhere.
- Commit submodule pointer updates in the workspace only when the combination is compatible (a known-good snapshot).
- Never commit in a submodule while on a detached HEAD — check out `main` first.
- Do not add Co-Authored-By trailers to commits.
