set -e
wasm-pack build --weak-refs --target bundler --out-name index --out-dir dist/bundler/main-wasm --no-pack -- --no-default-features  --features primitives
rm dist/bundler/main-wasm/.gitignore
wasm-pack build --weak-refs --target bundler --out-name index --out-dir dist/bundler/worker-wasm --no-pack -- --no-default-features  --features client
rm dist/bundler/worker-wasm/.gitignore
