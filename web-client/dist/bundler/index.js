import * as Comlink from 'comlink';
import { Address, Transaction } from './main-wasm/index.js';
import { clientFactory } from '../lib/browser/client-proxy.mjs';
import { setupMainThreadTransferHandlers } from '../lib/browser/transfer-handlers.mjs';

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
