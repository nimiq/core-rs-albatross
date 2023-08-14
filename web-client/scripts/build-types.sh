set -e
wasm-pack build --weak-refs --target web --out-name index --out-dir dist/types --no-pack -- --features client,primitives
find dist/types ! -name 'index.d.ts' -type f -exec rm {} +
