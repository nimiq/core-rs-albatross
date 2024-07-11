import * as Comlink from './comlink.min.mjs';
import init, { Address, CryptoUtils, Transaction } from './main-wasm/index.js';
import { clientFactory } from '../launcher/browser/client-proxy.mjs';
import { cryptoUtilsWorkerFactory } from '../launcher/browser/cryptoutils-worker-proxy.mjs';
import { setupMainThreadTransferHandlers } from '../launcher/browser/transfer-handlers.mjs';

setupMainThreadTransferHandlers(Comlink, {
    Address,
    Transaction,
});

const Client = clientFactory(
    () => new Worker(new URL('./worker.js', import.meta.url)),
    worker => Comlink.wrap(worker),
);

const CryptoUtilsWorker = cryptoUtilsWorkerFactory(
    () => new Worker(new URL('./crypto.js', import.meta.url)),
    worker => Comlink.wrap(worker),
);
for (const propName in CryptoUtilsWorker) {
    const prop = CryptoUtilsWorker[propName];
    if (typeof prop === 'function') {
        CryptoUtils[propName] = prop;
    }
}

export * from './main-wasm/index.js';
export { Client };
export * from '../lib/web/index.mjs';
export default init;
