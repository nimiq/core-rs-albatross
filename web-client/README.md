# Nimiq web client

This client is a very light client that only includes the necessary dependencies and constructs
to compile a client to WebAssembly and to run it in a web browser. This is a web-client intended
to be used in web browsers only (no WASI support). Although it can be built for other targets,
it will panic if it is executed outside a web browser.

## Building the example

To build the client, the recommended way is to use [`wasm-pack`](https://rustwasm.github.io/wasm-pack/).
This tool can be obtained using:

```
cargo install wasm-pack
```

or via their website. Once installed, the client can be built from this directory:

```sh
./scripts/build-web.sh && ./scripts/build-launcher.sh
```

The above command will compile the Rust code to WebAssembly and generate the corresponding JS
bindings required to run the client in a web browser.

After the client has been built, this directory can be served with a web server (e.g. `python3 -m http.server`)
and then the `/example/index.html` file can be loaded from e.g. http://localhost:8000/example/index.html.

## Publishing to NPM

> **Warning**
> You must be using wasm-pack version >= 0.11.0.

To publish this package to NPM, run this command:

```sh
./scripts/npm-publish.sh
```

This script builds the `web`, `bundler`, and `node` targets, the types and bindings and publishes the package.
