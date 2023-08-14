set -e
wasm-pack build --weak-refs --target web --out-name index --out-dir dist/web/main-wasm --no-pack -- --no-default-features  --features primitives
rm dist/web/main-wasm/.gitignore
wasm-pack build --weak-refs --target no-modules --out-name index --out-dir dist/web/worker-wasm --no-pack -- --no-default-features --features client
rm dist/web/worker-wasm/.gitignore
