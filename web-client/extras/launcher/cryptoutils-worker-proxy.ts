import type { Remote } from 'comlink';

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
    ['otpKdf'].forEach((method) => {
        proxy[method] = async function() {
            remote = remote || await startWorker(workerFactory, comlinkWrapper);
            return remote[method](...arguments);
        };
    });
    return proxy;
}

async function startWorker(workerFactory: () => Worker, comlinkWrapper: (worker: Worker) => Remote<CryptoUtilsWorker>) {
    const worker = workerFactory();

    // Wait for worker script to load
    await new Promise<void>((resolve) => {
        const readyListener = (event) => {
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

/** Abstractions over the differences between web workers and NodeJS worker_threads */

function addEventListener(worker: Worker, type: string, listener: (event: MessageEvent | {}) => void): void {
    const method = 'addListener' in worker ? 'addListener' : 'addEventListener';
    worker[method](type, listener);
}

function removeEventListener(worker: Worker, type: string, listener: (event: MessageEvent | {}) => void): void {
    const method = 'removeListener' in worker ? 'removeListener' : 'removeEventListener';
    worker[method](type, listener);
}

function getEventData(event: MessageEvent | {}): {} {
    return typeof event === 'object' && 'data' in event
        ? event.data
        : event;
}
