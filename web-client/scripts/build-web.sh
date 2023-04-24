wasm-pack build --weak-refs --target web --out-name index --out-dir dist/web/main-wasm &&
rm dist/web/main-wasm/.gitignore &&
rm dist/web/main-wasm/package.json &&
rm dist/web/main-wasm/README.md &&
wasm-pack build --weak-refs --target no-modules --out-name index --out-dir dist/web/worker-wasm &&
rm dist/web/worker-wasm/.gitignore &&
rm dist/web/worker-wasm/package.json &&
rm dist/web/worker-wasm/README.md
