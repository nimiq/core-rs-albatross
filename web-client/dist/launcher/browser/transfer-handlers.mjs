// launcher/transfer-handlers.ts
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
export {
  setupMainThreadTransferHandlers
};
