# Examples

## Browser

From the `web-client` directory (one up), you need to run

```sh
./scripts/build.sh --only web
```

Then serve the `web-client` directory and visit `http://localhost[:port]/example`. The action happens in the browser console.

## NodeJS

From the `web-client` directory (one up), you need to run

```sh
./scripts/build.sh --only nodejs
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

```sh
node index.mjs
```
