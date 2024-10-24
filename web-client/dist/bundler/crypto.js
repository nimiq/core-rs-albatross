// The worker has its own scope and no direct access to functions/objects of the
// global scope. We import the generated JS file to make `wasm_bindgen`
// available which we need to initialize our WASM code.
import * as Comlink from 'comlink';
import { CryptoUtils } from './crypto-wasm/index.js';

(async function init() {
    console.log('Initializing crypto WASM worker');

    Comlink.expose(CryptoUtils);

    self.postMessage('NIMIQ_ONLOAD');
})();
