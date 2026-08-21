# libnativeapi workspace

Aggregates all [libnativeapi](https://github.com/libnativeapi) repositories as git submodules, so cross-repo changes (core → codegen → bindings) can be made and tracked together.

## Repositories

| Submodule | Description |
| --- | --- |
| [nativeapi](https://github.com/libnativeapi/nativeapi) | C++ core library |
| [codegen](https://github.com/libnativeapi/codegen) | Binding code generator |
| [nativeapi-flutter](https://github.com/libnativeapi/nativeapi-flutter) | Flutter binding |
| [nativeapi-rust](https://github.com/libnativeapi/nativeapi-rust) | Rust binding |
| [nativeapi-swift](https://github.com/libnativeapi/nativeapi-swift) | Swift binding |
| [nativeapi-kotlin](https://github.com/libnativeapi/nativeapi-kotlin) | Kotlin binding |

## Getting started

```bash
git clone --recursive git@github.com:libnativeapi/workspace.git
```

Already cloned without `--recursive`?

```bash
git submodule update --init
```

## Common tasks

```bash
make status   # working-tree status of every submodule
make sync     # pull main in every submodule (fast-forward)
make bump     # stage updated submodule pointers for commit
```

Day-to-day development happens inside each submodule as if it were a standalone clone: commit and push from within the subdirectory as usual. The workspace repo only records snapshots — commit updated submodule pointers when the set of repos is in a known-good, compatible state.
