import type { Remote } from 'comlink';
import type { CryptoUtils } from '../../dist/types/bundler';

let remote: Remote<CryptoUtils> | undefined;

// class CryptoUtilsProxy {}

export function cryptoUtilsFactory(workerFactory: () => Worker, comlinkWrapper: (worker: Worker) => Remote<CryptoUtils>) {
    // Construct a proxy object that lazily initializes a crypto worker and forwards all known method calls to it.
    const proxy = {};
    ["computeHmacSha512", "computePBKDF2sha512", "otpKdf"].forEach((method) => {
        proxy[method] = async function() {
            remote = remote || await startWorker(workerFactory, comlinkWrapper);
            return remote[method](...arguments);
        };
    });
    return proxy;
}

async function startWorker(workerFactory: () => Worker, comlinkWrapper: (worker: Worker) => Remote<CryptoUtils>) {
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
