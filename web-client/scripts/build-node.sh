set -e
wasm-pack build --weak-refs --target nodejs --out-name index --out-dir dist/node/main-wasm --no-pack -- --no-default-features  --features primitives
rm dist/node/main-wasm/.gitignore
wasm-pack build --weak-refs --target nodejs --out-name index --out-dir dist/node/worker-wasm --no-pack -- --no-default-features  --features client
rm dist/node/worker-wasm/.gitignore
