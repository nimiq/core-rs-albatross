import * as Comlink from 'comlink';
import { Address, Transaction } from './main-wasm/index.js';
import { clientFactory } from '../client-proxy.js';
import { setupMainThreadTransferHandlers } from '../transfer-handlers.js';

setupMainThreadTransferHandlers(Comlink, {
    Address,
    Transaction,
});

const Client = clientFactory(
    () => new Worker(new URL('./worker.js', import.meta.url)),
    Comlink,
);

export * from './main-wasm/index.js';
export { Client };
