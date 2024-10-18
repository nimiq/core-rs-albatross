// The worker has its own scope and no direct access to functions/objects of the
// global scope. We import the generated JS file to make `wasm_bindgen`
// available which we need to initialize our WASM code.
const { parentPort } = require('node:worker_threads');
const Comlink = require('comlink');
const nodeEndpoint = require('comlink/dist/umd/node-adapter.js');
const { CryptoUtils } = require('./crypto-wasm/index.js');

(async function init() {
    console.log('Initializing crypto WASM worker');

    Comlink.expose(CryptoUtils, nodeEndpoint(parentPort));

    parentPort.postMessage('NIMIQ_ONLOAD');
})();
