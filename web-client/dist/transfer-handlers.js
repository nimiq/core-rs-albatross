export function setupMainThreadTransferHandlers(Comlink) {
    // Used by both main thread and in worker
    Comlink.transferHandlers.set('function', {
        canHandle: (obj) => typeof obj === "function",
        serialize(obj) {
            return Comlink.transferHandlers.get('proxy').serialize(obj);
        },
        // Cannot receive functions from worker
    });

    // Comlink.transferHandlers.set('WasmPointer', {
    //     canHandle: (_obj) => false, // Cannot send WasmPointers to worker, as they do not exist in the worker's memory
    //     deserialize(port) {
    //         return Comlink.transferHandlers.get('proxy').deserialize(port);
    //     },
    // });
}
