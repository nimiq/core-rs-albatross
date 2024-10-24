import { Worker } from 'node:worker_threads';
import Comlink from 'comlink';
import nodeEndpoint from 'comlink/dist/esm/node-adapter.min.mjs';
import { Address, CryptoUtils, Transaction } from './main-wasm/index.js';
import { clientFactory } from '../launcher/node/client-proxy.mjs';
import { cryptoUtilsWorkerFactory } from '../launcher/node/cryptoutils-worker-proxy.mjs';
import { setupMainThreadTransferHandlers } from '../launcher/node/transfer-handlers.mjs';

setupMainThreadTransferHandlers(Comlink, {
    Address,
    Transaction,
});

const Client = clientFactory(
    () => new Worker(new URL('./worker.mjs', import.meta.url)),
    worker => Comlink.wrap(nodeEndpoint(worker)),
);

const CryptoUtilsWorker = cryptoUtilsWorkerFactory(
    () => new Worker(new URL('./crypto.mjs', import.meta.url)),
    worker => Comlink.wrap(nodeEndpoint(worker)),
);
for (const propName in CryptoUtilsWorker) {
    const prop = CryptoUtilsWorker[propName];
    if (typeof prop === 'function') {
        CryptoUtils[propName] = prop;
    }
}

export * from './main-wasm/index.js';
export { Client };
export * from '../lib/node/index.mjs';
