# Nimiq Albatross Web Client

A very light Nimiq Proof-of-Stake client that runs in web browsers, compiled from Rust to WebAssembly.

> **Note**
> This web client is intended to be used in web browsers only (no WASI support either). Other webworker-enabled environments are not yet supported.

## 📦 Installation

You need to install this package from the `next` tag:

```sh
npm install @nimiq/core-web@next
```

or

```sh
yarn add @nimiq/core-web@next
```

## 🛠️ Usage

This package contains the WASM file bundled for two [targets](https://rustwasm.github.io/wasm-pack/book/commands/build.html#target): `bundler` and `web`. If you use any bundler in your code, like Webpack, you should probably use the `bundler` target (the default, also exported from the package root). If that doesn't work, or you require the `web` target for your use-case, jump to the [With ES Modules](#with-es-modules) section.

### With Bundlers

> **Note**
> For Webpack 5, you have to [enable the `asyncWebAssembly` experiment in your config](https://webpack.js.org/configuration/experiments/).

```js
// Import the package asynchronously:
const Nimiq = await import('@nimiq/core-web');

// Create a configuration builder:
const config = new Nimiq.ClientConfiguration();

// Connect to the Albatross Testnet:
// Optional, default is 'testalbatross'
config.network('testalbatross');

// Specify the seed nodes to initially connect to:
// Optional, default is ['/dns4/seed1.pos.nimiq-testnet.com/tcp/8443/wss']
config.seedNodes(['/dns4/seed1.pos.nimiq-testnet.com/tcp/8443/wss']);

// Change the lowest log level that is output to the console:
// Optional, default is 'info'
config.logLevel('info');

// Instantiate and launch the client:
const client = Nimiq.Client.create(config.build());
```

### With ES Modules

```js
// Import the package from the /web path:
import init, * as Nimiq from '@nimiq/core-web/web';

// Load and initialize the WASM file
init().then(() => {
    // Create a configuration builder:
    const config = new Nimiq.ClientConfiguration();

    // Connect to the Albatross Testnet:
    // Optional, default is 'testalbatross'
    config.network('testalbatross');

    // Specify the seed nodes to initially connect to:
    // Optional, default is ['/dns4/seed1.pos.nimiq-testnet.com/tcp/8443/wss']
    config.seedNodes(['/dns4/seed1.pos.nimiq-testnet.com/tcp/8443/wss']);

    // Change the lowest log level that is output to the console:
    // Optional, default is 'info'
    config.logLevel('debug');

    // Instantiate and launch the client:
    const client = Nimiq.Client.create(config.build());
});
```

## 🐛 Issues, Bugs and Feedback

This is an early version of the client code compiled to WebAssembly and as such there will be problems and friction, especially now that more people try it out in more environments than we could ever test ourselves.

If you encounter issues or you find a bug, please open an issue in our Github at https://github.com/nimiq/core-rs-albatross.

If you want to provide feedback or have questions about the client, our "Nimiq Coders Dojo" Telegram group and the [Community Forum](https://forum.nimiq.community/) are the right places for that.
