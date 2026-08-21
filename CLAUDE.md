# libnativeapi workspace

This is a workspace repo: every subdirectory is a git submodule of an independent repository under the `libnativeapi` GitHub org. Work inside a submodule is committed and pushed from that subdirectory; the workspace repo itself only tracks submodule pointers plus this shared tooling.

## Architecture

- `nativeapi` — the C++ core library. The source of truth for the native API surface (windows, tray icons, menus, displays, keyboard, dialogs, storage, etc.) with per-platform implementations (macOS/Windows/Linux).
- `codegen` — generates binding code for the language wrappers from the core API.
- `nativeapi-flutter`, `nativeapi-rust`, `nativeapi-swift`, `nativeapi-kotlin` — language bindings wrapping the core library. The Rust binding layers `crates/nativeapi` (safe API) over `crates/cnativeapi` (FFI).

A change to the core C++ API typically ripples: `nativeapi` → `codegen` → each binding. When making such a change, update all affected repos in the same session and verify each still builds.

## Conventions

- Each submodule tracks `branch = main`. Use `make sync` to fast-forward all of them; `make status` to see dirty state everywhere.
- Commit submodule pointer updates in the workspace only when the combination is compatible (a known-good snapshot).
- Never commit in a submodule while on a detached HEAD — check out `main` first.
- Do not add Co-Authored-By trailers to commits.
