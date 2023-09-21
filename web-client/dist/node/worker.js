// The worker has its own scope and no direct access to functions/objects of the
// global scope. We import the generated JS file to make `wasm_bindgen`
// available which we need to initialize our WASM code.
const { parentPort } = require('node:worker_threads');
const Comlink = require('comlink');
const nodeEndpoint = require('comlink/dist/umd/node-adapter.js');
const { w3cwebsocket } = require('websocket');
const { Client } = require('./worker-wasm/index.js');

// Provide a global WebSocket implementation, which is expected by the WASM code built for browsers.
global.WebSocket = w3cwebsocket;

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

    console.log('Initializing WASM worker');

    const client = await Client.create(config);
    Comlink.expose(client, nodeEndpoint(parentPort));
};

parentPort.addListener('message', async (event) => {
    const { type } = event;
    if (type !== 'NIMIQ_INIT') return;

    let { config } = event;
    if (!config || typeof config !== 'object') config = {};

    try {
        await init(config);
        parentPort.postMessage({ ok: true });
    } catch (error) {
        parentPort.postMessage({ ok: false, error: error.message, stack: error.stack });
    }
});

parentPort.postMessage('NIMIQ_ONLOAD');
console.log('Launched WASM worker, ready for init');
