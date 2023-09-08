# Examples

## Browser

From the `web-client` directory (one up), you need to run

```sh
./scripts/build-web.sh
```

to build the WASM files, and then

```sh
# This script uses `yarn`
./scripts/build-launcher.sh
```

to create the necessary launch files.

Then serve the `web-client` directory and visit `http://localhost[:port]/example`. The action happens in the browser console.

## NodeJS

From the `web-client` directory (one up), you need to run

```sh
./scripts/build-node.sh
```

to build the WASM files. Then go into `example/node` and run

```sh
yarn
```

to install the dependencies. Other package managers will also work, but the `preinstall` script uses `yarn`.

Once installation finishes, run the CommonJS example with

```sh
node index.js
```

and the ESM example with

```
node index.mjs
```
