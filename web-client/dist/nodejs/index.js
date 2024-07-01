const { join } = require('node:path');
const { Worker } = require('node:worker_threads');
const Comlink = require('comlink');
const nodeEndpoint = require('comlink/dist/umd/node-adapter.js');
const { Address, Transaction } = require('./main-wasm/index.js');
const { clientFactory } = require('../launcher/node/client-proxy.js');
const { setupMainThreadTransferHandlers } = require('../launcher/node/transfer-handlers.js');

setupMainThreadTransferHandlers(Comlink, {
    Address,
    Transaction,
});

const Client = clientFactory(
    () => new Worker(join(__dirname, './worker.js')),
    worker => Comlink.wrap(nodeEndpoint(worker)),
);

const reexports = require('./main-wasm/index.js');
Object.keys(reexports).forEach(key => {
  exports[key] = reexports[key]
});
exports.Client = Client;
