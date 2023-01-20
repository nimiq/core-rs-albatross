# Nimiq web client

This client is a very light client that only includes the necessary dependencies and constructs
to compile a client to WebAssembly and to run it in a web browser. This is a web-client intended
to be used in web browsers only (no WASI support). Although it can be built for other targets,
it will panic if it is executed outside a web browser.


To build the client, the recommended way is to use `wasm-pack`. This tool can be obtained using:

```
cargo install wasm-pack
```

Once installed, the client can be built using:

```
wasm-pack build --target web
```

The above command will compile the Rust code to WebAssembly and generate the corresponding JS
bindings required to run the client in a web browser.

After the client has been built, the root directory of this crate can be served with a web server
(e.g. `python3 -m http.server`) and then the `index.html` file can be loaded from the server.
