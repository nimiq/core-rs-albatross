# Nimiq web client

This client is a very light client that only includes the necessary dependencies and constructs
to compile a client to WebAssembly and to run it in a web browser. This is a web-client intended
to be used in web browsers only (no WASI support). It currently supports modern browsers and NodeJS
as its Javascript environments.

## Running the example

To run the example, first build the web-client by running the following script from this directory:

```sh
./scripts/build.sh
```

This script builds the `web`, `bundler`, and `node` targets and generates the corresponding JS
bindings required to run the client in a web browser.

After the client has been built, this directory can be served with a web server (e.g. `python3 -m http.server`)
and then the `/example/index.html` file can be loaded from e.g. http://localhost:8000/example/index.html.
