import * as Comlink from 'comlink';
import { Address, Transaction } from './main-wasm/index.js';
import { clientFactory } from '../launcher/browser/client-proxy.mjs';
import { setupMainThreadTransferHandlers } from '../launcher/browser/transfer-handlers.mjs';

setupMainThreadTransferHandlers(Comlink, {
    Address,
    Transaction,
});

const Client = clientFactory(
    () => new Worker(new URL('./worker.js', import.meta.url), { type : 'module' }),
    worker => Comlink.wrap(worker),
);

export * from './main-wasm/index.js';
export { Client };
export * from '../lib/browser/index.mjs';
