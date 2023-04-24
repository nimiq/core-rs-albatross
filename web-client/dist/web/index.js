import * as Comlink from './comlink.min.mjs';
import init from './main-wasm/index.js';
import { clientFactory } from '../client-proxy.js';
import { setupMainThreadTransferHandlers } from '../transfer-handlers.js';

setupMainThreadTransferHandlers(Comlink);

const Client = clientFactory(
    () => new Worker(new URL('./worker.js', import.meta.url)),
    Comlink,
);

export * from './main-wasm/index.js';
export { Client };
export default init;
