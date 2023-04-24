/**
 * @typedef {import("./bundler/index").PlainClientConfiguration} PlainClientConfiguration
 * @typedef {import("./bundler/index").Client} Client
 */

/**
 * @param {() => Worker} workerFactory
 * @param {*} Comlink
 * @returns {{create(config: PlainClientConfiguration): Promise<Client>}}
 */
export function clientFactory(workerFactory, Comlink) {
    // Expose Client "class" with one static `create` method
    return {
        /**
         *
         * @param {PlainClientConfiguration} config
         * @returns {Promise<Client>}
         */
        async create(config) {
            const worker = workerFactory();

            // Wait for worker script to load
            await new Promise((resolve) => {
                const readyListener = (event) => {
                    worker.removeEventListener('message', readyListener);
                    if (event.data === 'NIMIQ_ONLOAD') resolve();
                };
                worker.addEventListener('message', readyListener);
            });

            // Wrap the worker with Comlink, to transparently proxy any method calls on the client
            // instance to the worker.
            // ATTENTION: The receiver in the worker is only available after the initialization
            //            below completes.
            const client = Comlink.wrap(worker);

            // Send offline/online events from the main thread, as Chromium-based browsers
            // do not support these events in web workers:
            // https://bugs.chromium.org/p/chromium/issues/detail?id=114475
            window.addEventListener('offline', () => worker.postMessage('offline'));
            window.addEventListener('online', () => worker.postMessage('online'));

            // Create the client with the config in the worker
            console.log('Sending NIMIQ_INIT message to worker');
            worker.postMessage({
                type: 'NIMIQ_INIT',
                config,
            });
            await new Promise((resolve, reject) => {
                worker.addEventListener('message', (event) => {
                    if (!('ok' in event.data)) return;

                    if (event.data.ok === true) resolve();
                    if (event.data.ok === false) {
                        const error = new Error(event.data.error);
                        error.stack = event.data.stack;
                        reject(error);
                    }
                });
            });

            console.log('Have worker client');
            return client;
        },
    };
};
