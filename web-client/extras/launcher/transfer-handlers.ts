import type { TransferHandler, transferHandlers } from 'comlink';

interface Serializable {
    serialize(): Uint8Array;
}

export function setupMainThreadTransferHandlers(
    Comlink: { transferHandlers: typeof transferHandlers },
    classes: { Address: any, Transaction: any },
) {
    Comlink.transferHandlers.set('function', {
        canHandle: (obj) => typeof obj === 'function',
        serialize(obj) {
            return Comlink.transferHandlers.get('proxy')!.serialize(obj);
        },
        // deserialize(plain) {}, // Cannot receive functions from worker
    } as TransferHandler<Function, MessagePort>);

    // Serialize some class instances to allow them to be used as arguments in Client methods
    function canBeSerialized(obj: unknown): obj is Serializable {
        return obj instanceof classes.Address
            || obj instanceof classes.Transaction;
    }

    // @ts-expect-error Cannot force `canHandle` to be the correct type
    Comlink.transferHandlers.set('plain', {
        canHandle: (obj) =>
            canBeSerialized(obj)
            || (Array.isArray(obj) && obj.some(item => canBeSerialized(item))),
        serialize(obj) {
            if (Array.isArray(obj)) {
                return [obj.map(item => canBeSerialized(item) ? item.serialize() : item), []];
            } else {
                return [obj.serialize(), []];
            }
        },
        // deserialize(plain) {}, // Cannot receive class instances from worker
    } as TransferHandler<Serializable | Serializable[], object>);
}
