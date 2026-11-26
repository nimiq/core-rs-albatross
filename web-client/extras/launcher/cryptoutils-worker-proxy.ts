import type { Remote } from 'comlink';
import { addEventListener, getEventData, removeEventListener } from './worker-utils';

interface CryptoUtilsWorker {
    otpKdf(message: Uint8Array, key: Uint8Array, salt: Uint8Array, iterations: number): Promise<Uint8Array>;
}

let remote: Remote<CryptoUtilsWorker> | undefined;

export function cryptoUtilsWorkerFactory(
    workerFactory: () => Worker,
    comlinkWrapper: (worker: Worker) => Remote<CryptoUtilsWorker>,
) {
    // Construct a proxy object that lazily initializes a crypto worker and forwards all known method calls to it.
    const proxy: Record<string, Function> = {};
    (['otpKdf'] as const).forEach((method) => {
        proxy[method] = async function(...args: Parameters<CryptoUtilsWorker[typeof method]>) {
            remote = remote || await startWorker(workerFactory, comlinkWrapper);
            return remote[method](...args);
        };
    });
    return proxy;
}

async function startWorker(workerFactory: () => Worker, comlinkWrapper: (worker: Worker) => Remote<CryptoUtilsWorker>) {
    const worker = workerFactory();

    // Wait for worker script to load
    await new Promise<void>((resolve) => {
        const readyListener = (event: MessageEvent | {}) => {
            removeEventListener(worker, 'message', readyListener);
            if (getEventData(event) === 'NIMIQ_ONLOAD') resolve();
        };
        addEventListener(worker, 'message', readyListener);
    });

    // Wrap the worker with Comlink, to transparently proxy any method calls on the CryptoUtils
    // instance to the worker.
    console.debug('Have crypto worker remote');
    return comlinkWrapper(worker);
}
