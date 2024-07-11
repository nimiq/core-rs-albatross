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

// launcher/transfer-handlers.ts
var transfer_handlers_exports = {};
__export(transfer_handlers_exports, {
  setupMainThreadTransferHandlers: () => setupMainThreadTransferHandlers
});
module.exports = __toCommonJS(transfer_handlers_exports);
function setupMainThreadTransferHandlers(Comlink, classes) {
  Comlink.transferHandlers.set("function", {
    canHandle: (obj) => typeof obj === "function",
    serialize(obj) {
      return Comlink.transferHandlers.get("proxy").serialize(obj);
    }
    // deserialize(plain) {}, // Cannot receive functions from worker
  });
  function canBeSerialized(obj) {
    return obj instanceof classes.Address || obj instanceof classes.Transaction;
  }
  Comlink.transferHandlers.set("plain", {
    canHandle: (obj) => canBeSerialized(obj) || Array.isArray(obj) && obj.some((item) => canBeSerialized(item)),
    serialize(obj) {
      if (Array.isArray(obj)) {
        return [obj.map((item) => canBeSerialized(item) ? item.serialize() : item), []];
      } else {
        return [obj.serialize(), []];
      }
    }
    // deserialize(plain) {}, // Cannot receive class instances from worker
  });
}
// Annotate the CommonJS export names for ESM import in node:
0 && (module.exports = {
  setupMainThreadTransferHandlers
});
