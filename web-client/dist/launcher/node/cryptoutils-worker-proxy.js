"use strict";
var __defProp = Object.defineProperty;
var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
var __getOwnPropNames = Object.getOwnPropertyNames;
var __hasOwnProp = Object.prototype.hasOwnProperty;
var __export = (target, all) => {
  for (var name in all)
    __defProp(target, name, { get: all[name], enumerable: true });
};
var __copyProps = (to, from, except, desc) => {
  if (from && typeof from === "object" || typeof from === "function") {
    for (let key of __getOwnPropNames(from))
      if (!__hasOwnProp.call(to, key) && key !== except)
        __defProp(to, key, { get: () => from[key], enumerable: !(desc = __getOwnPropDesc(from, key)) || desc.enumerable });
  }
  return to;
};
var __toCommonJS = (mod) => __copyProps(__defProp({}, "__esModule", { value: true }), mod);

// launcher/cryptoutils-worker-proxy.ts
var cryptoutils_worker_proxy_exports = {};
__export(cryptoutils_worker_proxy_exports, {
  cryptoUtilsWorkerFactory: () => cryptoUtilsWorkerFactory
});
module.exports = __toCommonJS(cryptoutils_worker_proxy_exports);
var remote;
function cryptoUtilsWorkerFactory(workerFactory, comlinkWrapper) {
  const proxy = {};
  ["otpKdf"].forEach((method) => {
    proxy[method] = async function() {
      remote = remote || await startWorker(workerFactory, comlinkWrapper);
      return remote[method](...arguments);
    };
  });
  return proxy;
}
async function startWorker(workerFactory, comlinkWrapper) {
  const worker = workerFactory();
  await new Promise((resolve) => {
    const readyListener = (event) => {
      removeEventListener(worker, "message", readyListener);
      if (getEventData(event) === "NIMIQ_ONLOAD") resolve();
    };
    addEventListener(worker, "message", readyListener);
  });
  console.debug("Have crypto worker remote");
  return comlinkWrapper(worker);
}
function addEventListener(worker, type, listener) {
  const method = "addListener" in worker ? "addListener" : "addEventListener";
  worker[method](type, listener);
}
function removeEventListener(worker, type, listener) {
  const method = "removeListener" in worker ? "removeListener" : "removeEventListener";
  worker[method](type, listener);
}
function getEventData(event) {
  return typeof event === "object" && "data" in event ? event.data : event;
}
// Annotate the CommonJS export names for ESM import in node:
0 && (module.exports = {
  cryptoUtilsWorkerFactory
});
