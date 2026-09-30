# Nimiq web client

This client is a very light client that only includes the necessary dependencies and constructs
to compile a client to WebAssembly and to run it in a web browser. This is a web-client intended
to be used in web browsers only (no WASI support). It currently supports modern browsers and NodeJS
as its Javascript environments.

## Transaction status and execution

`PlainTransactionDetails.state` describes inclusion and finality, while `executionResult`
describes whether execution succeeded. A transaction can have `state: "confirmed"` and
`executionResult: false`: it was finalized, but execution failed. Check both fields before
reporting a successful payment. `executionResult` is `true` for successful execution and `false`
for failed execution. If `sendTransaction()` returns `new` without an execution result, the client
has not observed inclusion yet. The transaction may still have been included; do not assume
failure or success from that response.

`getTransaction(hash)` fetches an included transaction from the blockchain with a known
`executionResult`. It returns an error if no matching transaction is found or the lookup fails.
An error alone does not prove the transaction was not included.

## Running the example

### Requisites for every OS

You must have Node installed. Follow [these instructions](https://nodejs.org/en/download/package-manager) to install it.

### Requisites for MAC-OS systems

On MAC-OS there are special requirements.

1. Install and add LLVM Clang to the path (The Clang shipped with your system doesn't have support enabled for
   `wasm32_unknown_unknown`):
   1. Install LLVM Clang: `brew install llvm`.
   2. Add it to the `PATH`: `export PATH="/opt/homebrew/opt/llvm/bin:$PATH"` (and remember to add it to your
      `.zshrc` or `.bashrc` for future uses).
   3. Verify the installation: `llvm-config --version`.
2. Install GNU sed (The sed shipped with your system is very old and the script below assumes newer versions):
   1. Install `sed`: `brew install gnu-sed`.
   2. Add it to the `PATH`: `export PATH="/opt/homebrew/opt/gnu-sed/libexec/gnubi:$PATH"` (and remember to add it to your
      `.zshrc` or `.bashrc` for future uses).
   3. Verify the installation: `sed --version`.


### Steps for every system

To run the example, first build the web-client by running the following script from this directory:

```sh
./scripts/build.sh --only web,types
```

This script builds the `web` wasm-bindgen target and generates the corresponding JS bindings required
to run the client in a web browser.

After the client has been built, this directory can be served with a web server (e.g. `python3 -m http.server`)
and then the `/example/index.html` file can be loaded from e.g. http://localhost:8000/example/index.html.
