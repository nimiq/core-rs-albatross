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

Comlink.transferHandlers.set('plain', {
    canHandle: (_obj) => false, // Cannot send class instances to main thread
    deserialize(plain) {
        return plain;
    },
});

let initialized = false;

async function init(config) {
    if (initialized) throw new Error('Already initialized');
    initialized = true;

    console.log('Initializing client WASM worker');

    const client = await Client.create(config);
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
    } catch (error) {
        self.postMessage({ ok: false, error: error.message, stack: error.stack });
    }
});

self.postMessage('NIMIQ_ONLOAD');
console.debug('Launched client WASM worker, ready for init');
