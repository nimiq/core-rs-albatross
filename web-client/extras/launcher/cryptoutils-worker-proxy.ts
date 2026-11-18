import type { Remote } from 'comlink';
import type { Worker as NodeWorker } from 'node:worker_threads';

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

/** Abstractions over the differences between web workers and NodeJS worker_threads */

// In NodeJS, the worker factory returns a worker_threads Worker, even though it's typed as a web Worker.
type AnyWorker = Worker | NodeWorker;

function addEventListener(worker: AnyWorker, type: string, listener: (event: MessageEvent | {}) => void): void {
    if ('addListener' in worker) worker.addListener(type, listener);
    else worker.addEventListener(type, listener);
}

function removeEventListener(worker: AnyWorker, type: string, listener: (event: MessageEvent | {}) => void): void {
    if ('removeListener' in worker) worker.removeListener(type, listener);
    else worker.removeEventListener(type, listener);
}

function getEventData(event: MessageEvent | {}): {} {
    return typeof event === 'object' && 'data' in event
        ? event.data
        : event;
}
