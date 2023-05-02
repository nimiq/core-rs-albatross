export function setupMainThreadTransferHandlers(Comlink, classes) {
    Comlink.transferHandlers.set('function', {
        canHandle: (obj) => typeof obj === "function",
        serialize(obj) {
            return Comlink.transferHandlers.get('proxy').serialize(obj);
        },
        // deserialize(plain) {}, // Cannot receive functions from worker
    });

    // Serialize some class instances to allow them to be used as arguments in Client methods
    function canBeSerializedToPlain(obj) {
        return obj instanceof classes.Address
            || obj instanceof classes.Transaction;
    }

    Comlink.transferHandlers.set('plain', {
        canHandle: (obj) => canBeSerializedToPlain(obj)
            || (Array.isArray(obj) && obj.some(item => canBeSerializedToPlain(item))),
        serialize(obj) {
            if (Array.isArray(obj)) {
                return [obj.map(item => canBeSerializedToPlain(item) ? item.toPlain() : item), []];
            } else {
                return [obj.toPlain(), []];
            }
        },
        // deserialize(plain) {}, // Cannot receive class instances from worker
    });

    // Comlink.transferHandlers.set('WasmPointer', {
    //     canHandle: (_obj) => false, // Cannot send WasmPointers to worker, as they do not exist in the worker's memory
    //     deserialize(port) {
    //         return Comlink.transferHandlers.get('proxy').deserialize(port);
    //     },
    // });
}
