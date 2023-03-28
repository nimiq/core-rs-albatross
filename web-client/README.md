# Nimiq web client

This client is a very light client that only includes the necessary dependencies and constructs
to compile a client to WebAssembly and to run it in a web browser. This is a web-client intended
to be used in web browsers only (no WASI support). Although it can be built for other targets,
it will panic if it is executed outside a web browser.

## Building the example

To build the client, the recommended way is to use [`wasm-pack`](https://rustwasm.github.io/wasm-pack/). This tool can be obtained using:

```
cargo install wasm-pack
```

or via their website. Once installed, the client can be built from this directory:

```bash
wasm-pack build --target web --weak-refs
```

> **Note**
> The `--weak-refs` flag is available since wasm-pack v0.11.0.
> For older versions prefix the command with `WASM_BINDGEN_WEAKREF=1` instead.
> Check your installed version with `wasm-pack --version`.

The above command will compile the Rust code to WebAssembly and generate the corresponding JS
bindings required to run the client in a web browser.

After the client has been built, this directory can be served with a web server
(e.g. `python3 -m http.server`) and then the `index.html` file can be loaded from e.g. http://localhost:8000.

## Publishing to NPM

> **Warning**
> You must be using wasm-pack version >= 0.11.0.

To publish this package to NPM, run this command:

```bash
wasm-pack build --release --weak-refs --target bundler --out-dir dist/bundler --out-name index && \
wasm-pack build --release --weak-refs --target web --out-dir dist/web --out-name index && \
cd dist && \
npm publish --tag next && \
cd ..
```
