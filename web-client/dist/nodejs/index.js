const { join } = require('node:path');
const { Worker } = require('node:worker_threads');
const Comlink = require('comlink');
const nodeEndpoint = require('comlink/dist/umd/node-adapter.js');
const { Address, CryptoUtils, Transaction } = require('./main-wasm/index.js');
const { clientFactory } = require('../launcher/node/client-proxy.js');
const { cryptoUtilsWorkerFactory } = require('../launcher/node/cryptoutils-worker-proxy.js');
const { setupMainThreadTransferHandlers } = require('../launcher/node/transfer-handlers.js');

setupMainThreadTransferHandlers(Comlink, {
    Address,
    Transaction,
});

const Client = clientFactory(
    () => new Worker(join(__dirname, './worker.js')),
    worker => Comlink.wrap(nodeEndpoint(worker)),
);

const CryptoUtilsWorker = cryptoUtilsWorkerFactory(
    () => new Worker(join(__dirname, './crypto.js')),
    worker => Comlink.wrap(nodeEndpoint(worker)),
);
for (const propName in CryptoUtilsWorker) {
  const prop = CryptoUtilsWorker[propName];
  if (typeof prop === 'function') {
      CryptoUtils[propName] = prop;
  }
}

const wasmexports = require('./main-wasm/index.js');
Object.keys(wasmexports).forEach(key => {
  exports[key] = wasmexports[key]
});
exports.Client = Client;
const libexports = require('../lib/node/index.js');
Object.keys(libexports).forEach(key => {
  exports[key] = libexports[key]
});
