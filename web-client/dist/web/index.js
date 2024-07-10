import * as Comlink from './comlink.min.mjs';
import { Address, Transaction } from './main-wasm/index.js';
import { clientFactory } from '../launcher/browser/client-proxy.mjs';
import { cryptoUtilsFactory } from '../launcher/browser/cryptoutils-proxy.mjs';
import { setupMainThreadTransferHandlers } from '../launcher/browser/transfer-handlers.mjs';

setupMainThreadTransferHandlers(Comlink, {
    Address,
    Transaction,
});

const Client = clientFactory(
    () => new Worker(new URL('./worker.js', import.meta.url)),
    worker => Comlink.wrap(worker),
);

const CryptoUtils = cryptoUtilsFactory(
    () => new Worker(new URL('./crypto.js', import.meta.url)),
    worker => Comlink.wrap(worker),
);

export * from './main-wasm/index.js';
export { Client, CryptoUtils };
export * from '../lib/browser/index.mjs';
