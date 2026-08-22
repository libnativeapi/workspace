# libnativeapi workspace

This is a workspace repo for the [libnativeapi](https://github.com/libnativeapi) project family. `core/` and `bindings/*` are git submodules of independent repositories; the code generator (`tools/codegen/`) and the `./codegen` script live directly in this repo. Work inside a submodule is committed and pushed from that subdirectory; the workspace repo tracks submodule pointers, the code generator, and shared tooling.

## Layout

```
core/               # submodule: nativeapi — the C++ core library
bindings/
├── flutter/        # submodule: nativeapi-flutter
├── rust/           # submodule: nativeapi-rust
├── swift/          # submodule: nativeapi-swift
├── kotlin/         # submodule: nativeapi-kotlin
└── csharp/         # submodule: nativeapi-csharp
tools/codegen/      # in-repo Rust workspace: the code generator
codegen             # Python entry point orchestrating the generators
```

## Architecture

- `core` — the C++ core library (repo: `nativeapi`). The source of truth for the native API surface (windows, tray icons, menus, displays, keyboard, dialogs, storage, etc.) with per-platform implementations (macOS/Windows/Linux).
- `tools/codegen` — three crates: `shared` (libclang parser, IR, naming), `capi` (C ABI + umbrella header), `bindings` (Rust/Swift/Dart/C# generators, consuming the IR JSON emitted by `capi`). Only `capi` depends on libclang. See tools/codegen/README.md.
- `bindings/*` — language bindings wrapping the core library (repos: `nativeapi-<lang>`). Each embeds the core repo as a nested submodule (`cxx_impl`, or `Sources/CNativeAPI` for Swift). The Rust binding layers `crates/nativeapi` (safe API) over `crates/cnativeapi` (FFI).

## Code generation

Always drive the generators through `./codegen` at the workspace root:

- `./codegen` — full run: C ABI, then all bindings
- `./codegen capi` / `./codegen bindings [--lang rust,swift,dart,csharp]`
- `./codegen check` — read-only verification, non-zero exit when stale (CI mode)
- `./codegen sync [-m "msg"] [--push]` — full downstream propagation, see below

Generated files start with `// AUTO-GENERATED. DO NOT EDIT.` — change the C++ headers in `core/src/` and regenerate instead of editing outputs. Files without that banner are hand-written and never overwritten. The header list (`API_HEADERS`) lives in `tools/codegen/shared/src/lib.rs`.

## Changing the core API

A core change ripples to every binding. After editing headers in `core`, run:

```bash
./codegen sync -m "<core commit message>"
```

It regenerates everything, updates each binding's embedded core submodule (fetched from the local `core/`, so no push is required first), reruns `bindgen` (Rust raw FFI) and the flutter repo's `codegen.py` (umbrella headers + ffigen), then commits core, each changed binding (`Sync with core <sha>`), and the workspace submodule pointers. Add `--push` to publish in dangling-safe order (core → bindings → workspace).

Manual follow-ups sync cannot do (details in tools/codegen/README.md):

- New handle types need an `IdTypeTag<T>` entry in `core/src/foundation/id_allocator.h` (append only; a miss is a compile error, not silent).
- New Rust modules need a `pub mod` declaration in `bindings/rust/crates/nativeapi/src/lib.rs`.

## Conventions

- Each submodule tracks `branch = main`. Use `make sync` to fast-forward all of them; `make status` to see dirty state everywhere; `make bump` to stage pointer updates.
- Commit workspace submodule pointer updates only when the combination is compatible (a known-good snapshot).
- Never commit in a submodule while on a detached HEAD — check out `main` first (`./codegen sync` enforces this).
- Do not add Co-Authored-By trailers to commits.
