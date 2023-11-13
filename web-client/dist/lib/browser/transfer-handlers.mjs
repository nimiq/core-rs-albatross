// transfer-handlers.ts
function setupMainThreadTransferHandlers(Comlink, classes) {
  Comlink.transferHandlers.set("function", {
    canHandle: (obj) => typeof obj === "function",
    serialize(obj) {
      return Comlink.transferHandlers.get("proxy").serialize(obj);
    }
    // deserialize(plain) {}, // Cannot receive functions from worker
  });
  function canBeSerializedToPlain(obj) {
    return obj instanceof classes.Address || obj instanceof classes.Transaction;
  }
  Comlink.transferHandlers.set("plain", {
    canHandle: (obj) => canBeSerializedToPlain(obj) || Array.isArray(obj) && obj.some((item) => canBeSerializedToPlain(item)),
    serialize(obj) {
      if (Array.isArray(obj)) {
        return [obj.map((item) => canBeSerializedToPlain(item) ? item.toPlain() : item), []];
      } else {
        return [obj.toPlain(), []];
      }
    }
    // deserialize(plain) {}, // Cannot receive class instances from worker
  });
}
export {
  setupMainThreadTransferHandlers
};
