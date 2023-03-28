wasm-pack build --release --weak-refs --target bundler --out-dir dist/bundler --out-name index &&
wasm-pack build --release --weak-refs --target web --out-dir dist/web --out-name index &&
cd dist &&
npm publish --tag next &&
cd ..
