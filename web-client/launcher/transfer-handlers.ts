import type { transferHandlers, TransferHandler } from 'comlink';

interface Plainable {
    toPlain(): object,
}

export function setupMainThreadTransferHandlers(Comlink: { transferHandlers: typeof transferHandlers }, classes: { Address: any, Transaction: any }) {
    Comlink.transferHandlers.set('function', {
        canHandle: (obj) => typeof obj === "function",
        serialize(obj) {
            return Comlink.transferHandlers.get('proxy')!.serialize(obj);
        },
        // deserialize(plain) {}, // Cannot receive functions from worker
    } as TransferHandler<Function, MessagePort>);

    // Serialize some class instances to allow them to be used as arguments in Client methods
    function canBeSerializedToPlain(obj: unknown): obj is Plainable {
        return obj instanceof classes.Address
            || obj instanceof classes.Transaction;
    }

    // @ts-expect-error Cannot force `canHandle` to be the correct type
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
    } as TransferHandler<Plainable | Plainable[], object>);
}
