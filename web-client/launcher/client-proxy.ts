import type { Remote } from 'comlink';
import type { Client, PlainClientConfiguration } from '../dist/types/index';

export function clientFactory(workerFactory: () => Worker, comlinkWrapper: (worker: Worker) => Remote<Client>): {
    create(config: PlainClientConfiguration): Promise<Client>,
} {
    // Expose Client stub with one static `create` method that launches a client instance in a web worker and returns a proxy to it.
    return {
        async create(config: PlainClientConfiguration): Promise<Client> {
            const worker = workerFactory();

            // Wait for worker script to load
            await new Promise<void>((resolve) => {
                const readyListener = (event) => {
                    removeEventListener(worker, 'message', readyListener);
                    if (getEventData(event) === 'NIMIQ_ONLOAD') resolve();
                };
                addEventListener(worker, 'message', readyListener);
            });
            console.debug('WASM worker loaded');

            // Wrap the worker with Comlink, to transparently proxy any method calls on the client
            // instance to the worker.
            // ATTENTION: The receiver in the worker is only available after the initialization
            //            below completes.
            const client = comlinkWrapper(worker);

            if (typeof window !== 'undefined') {
                // Send offline/online/visible events from the main thread, as Chromium-based browsers
                // do not support these events in web workers:
                // https://bugs.chromium.org/p/chromium/issues/detail?id=114475
                window.addEventListener('offline', () => worker.postMessage('offline'));
                window.addEventListener('online', () => worker.postMessage('online'));
            }

            if (typeof document !== 'undefined') {
                document.addEventListener('visibilitychange', () => {
                    if (document.visibilityState === "visible") {
                        worker.postMessage('visible');
                    }
                });
            }

            // Create the client with the config in the worker
            console.debug('Sending NIMIQ_INIT message to worker');
            worker.postMessage({
                type: 'NIMIQ_INIT',
                config,
            });
            await new Promise<void>((resolve, reject) => {
                addEventListener(worker, 'message', (event) => {
                    const eventData = getEventData(event);

                    if (!('ok' in eventData)) return;

                    if (eventData.ok === true) resolve();
                    if (eventData.ok === false && 'error' in eventData && typeof eventData.error === 'string') {
                        const error = new Error(eventData.error);
                        if ('stack' in eventData && typeof eventData.stack === 'string') {
                            error.stack = eventData.stack;
                        }
                        reject(error);
                    }
                });
            });

            console.debug('Have worker client');
            return client;
        },
    };
};

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
