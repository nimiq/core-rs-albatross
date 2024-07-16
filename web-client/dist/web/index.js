import * as Comlink from './comlink.min.mjs';
import { Address, Transaction } from './main-wasm/index.js';
import { clientFactory } from '../launcher/browser/client-proxy.mjs';
import { setupMainThreadTransferHandlers } from '../launcher/browser/transfer-handlers.mjs';

setupMainThreadTransferHandlers(Comlink, {
    Address,
    Transaction,
});

const Client = clientFactory(
    () => new Worker(new URL('./worker.js', import.meta.url)),
    worker => Comlink.wrap(worker),
);

export { default } from './main-wasm/index.js';
export * from './main-wasm/index.js';
export { Client };
export * from '../lib/browser/index.mjs';
