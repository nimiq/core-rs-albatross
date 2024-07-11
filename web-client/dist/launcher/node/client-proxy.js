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

// launcher/client-proxy.ts
var client_proxy_exports = {};
__export(client_proxy_exports, {
  clientFactory: () => clientFactory
});
module.exports = __toCommonJS(client_proxy_exports);
function clientFactory(workerFactory, comlinkWrapper) {
  return {
    async create(config) {
      const worker = workerFactory();
      await new Promise((resolve) => {
        const readyListener = (event) => {
          removeEventListener(worker, "message", readyListener);
          if (getEventData(event) === "NIMIQ_ONLOAD") resolve();
        };
        addEventListener(worker, "message", readyListener);
      });
      console.debug("Client WASM worker loaded");
      const client = comlinkWrapper(worker);
      if (typeof window !== "undefined") {
        window.addEventListener("offline", () => worker.postMessage("offline"));
        window.addEventListener("online", () => worker.postMessage("online"));
      }
      if (typeof document !== "undefined") {
        document.addEventListener("visibilitychange", () => {
          if (document.visibilityState === "visible") {
            worker.postMessage("visible");
          }
        });
      }
      console.debug("Sending NIMIQ_INIT message to client worker");
      worker.postMessage({
        type: "NIMIQ_INIT",
        config
      });
      await new Promise((resolve, reject) => {
        addEventListener(worker, "message", (event) => {
          const eventData = getEventData(event);
          if (!("ok" in eventData)) return;
          if (eventData.ok === true) resolve();
          if (eventData.ok === false && "error" in eventData && typeof eventData.error === "string") {
            const error = new Error(eventData.error);
            if ("stack" in eventData && typeof eventData.stack === "string") {
              error.stack = eventData.stack;
            }
            reject(error);
          }
        });
      });
      console.debug("Have client worker remote");
      return client;
    }
  };
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
  clientFactory
});
