# elfvis

In-browser ELF binary size treemap visualizer. Drop a firmware ELF (compiled
with `-g`) and instantly see what's consuming flash — by source directory, file,
and symbol.

Supports ARM, RISC-V, Xtensa, and x86 ELF binaries.

## Build

```bash
wasm-pack build --target web --out-dir www/pkg
```

Serve `www/` with any static file server.

## Develop

```bash
cargo test          # Run core logic tests (native)
wasm-pack build --target web --out-dir www/pkg --dev
python3 -m http.server -d www 8080
```
