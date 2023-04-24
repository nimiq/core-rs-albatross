wasm-pack build --weak-refs --target bundler --out-name index --out-dir dist/bundler/main-wasm &&
rm dist/bundler/main-wasm/.gitignore &&
rm dist/bundler/main-wasm/package.json &&
rm dist/bundler/main-wasm/README.md &&
wasm-pack build --weak-refs --target bundler --out-name index --out-dir dist/bundler/worker-wasm &&
rm dist/bundler/worker-wasm/.gitignore &&
rm dist/bundler/worker-wasm/package.json &&
rm dist/bundler/worker-wasm/README.md
