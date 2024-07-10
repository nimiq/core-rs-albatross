import { Worker } from 'node:worker_threads';
import Comlink from 'comlink';
import nodeEndpoint from 'comlink/dist/esm/node-adapter.min.mjs';
import wasm from './main-wasm/index.js';
import { clientFactory } from '../launcher/node/client-proxy.mjs';
import { cryptoUtilsFactory } from '../launcher/node/cryptoutils-proxy.mjs';
import { setupMainThreadTransferHandlers } from '../launcher/node/transfer-handlers.mjs';

setupMainThreadTransferHandlers(Comlink, {
    Address: wasm.Address,
    Transaction: wasm.Transaction,
});

const Client = clientFactory(
    () => new Worker(new URL('./worker.mjs', import.meta.url)),
    worker => Comlink.wrap(nodeEndpoint(worker)),
);

const CryptoUtils = cryptoUtilsFactory(
    () => new Worker(new URL('./crypto.mjs', import.meta.url)),
    worker => Comlink.wrap(nodeEndpoint(worker)),
);

export * from './main-wasm/index.js';
export { Client, CryptoUtils };
export * from '../lib/node/index.mjs';
