/** Abstractions over the differences between web workers and NodeJS worker_threads */

import type { Worker as NodeWorker } from 'node:worker_threads';

// In NodeJS, the worker factory returns a worker_threads Worker, even though it's typed as a web Worker.
type AnyWorker = Worker | NodeWorker;

export function addEventListener(
    worker: AnyWorker,
    type: string,
    listener: (event: MessageEvent | {}) => void,
): void {
    if ('addListener' in worker) worker.addListener(type, listener);
    else worker.addEventListener(type, listener);
}

export function removeEventListener(
    worker: AnyWorker,
    type: string,
    listener: (event: MessageEvent | {}) => void,
): void {
    if ('removeListener' in worker) worker.removeListener(type, listener);
    else worker.removeEventListener(type, listener);
}

export function getEventData(event: MessageEvent | {}): {} {
    return typeof event === 'object' && 'data' in event
        ? event.data
        : event;
}
