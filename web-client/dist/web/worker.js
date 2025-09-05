// The worker has its own scope and no direct access to functions/objects of the
// global scope. We import the generated JS file to make `wasm_bindgen`
// available which we need to initialize our WASM code.
importScripts(
    './comlink.min.js',
    './worker-wasm/index.js',
);

const { Client } = wasm_bindgen;

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

    // Load the wasm file by awaiting the Promise returned by `wasm_bindgen`.
    await wasm_bindgen('./worker-wasm/index_bg.wasm');

    const client = await Client.create(config);
    Comlink.expose(client);
};

self.addEventListener('message', async (event) => {
    const data = event.data;

    if (data === 'NIMIQ_CHECKREADY') {
        self.postMessage('NIMIQ_READY');
        return;
    }

    if (data.type !== 'NIMIQ_INIT') return;

    let config = data.config;
    if (!config || typeof config !== 'object') config = {};

    try {
        await init(config);
        self.postMessage({ ok: true });
    } catch (error) {
        self.postMessage({ ok: false, error: error.message, stack: error.stack });
    }
});

console.debug('Client WASM worker ready');
