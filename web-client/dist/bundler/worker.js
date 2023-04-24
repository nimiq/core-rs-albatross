// The worker has its own scope and no direct access to functions/objects of the
// global scope. We import the generated JS file to make `wasm_bindgen`
// available which we need to initialize our WASM code.
import * as Comlink from 'comlink';
import { Client } from './worker-wasm/index.js';

// Defined both here and in main thread exports.js
Comlink.transferHandlers.set('function', {
    canHandle: (_obj) => false, // Cannot send functions to main thread
    deserialize(port) {
        return Comlink.transferHandlers.get('proxy').deserialize(port);
    },
});

// Comlink.transferHandlers.set('WasmPointer', {
//     canHandle: (obj) => obj instanceof Transaction || obj instanceof TransactionBuilder,
//     serialize(obj) {
//         return Comlink.transferHandlers.get('proxy').serialize(obj);
//     },
//     // Cannot receive functions from worker
// });

let initialized = false;

async function init(config) {
    if (initialized) throw new Error('Already initialized');
    initialized = true;

    console.log('Initializing WASM worker');

    const client = await Client.create(config);
    // self.client = client; // Prevent garbage collection
    Comlink.expose(client);
};

self.addEventListener('message', async (event) => {
    const { type } = event.data;
    if (type !== 'NIMIQ_INIT') return;

    let { config } = event.data;
    if (!config || typeof config !== 'object') config = {};

    try {
        await init(config);
        self.postMessage({ ok: true });
        // console.log('OK');
    } catch (error) {
        self.postMessage({ ok: false, error: error.message, stack: error.stack });
        // console.error(error);
    }
});

self.postMessage('NIMIQ_ONLOAD');
console.log('Loaded WASM worker, ready for init');
