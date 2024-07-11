// launcher/cryptoutils-worker-proxy.ts
var remote;
function cryptoUtilsWorkerFactory(workerFactory, comlinkWrapper) {
  const proxy = {};
  ["otpKdf"].forEach((method) => {
    proxy[method] = async function() {
      remote = remote || await startWorker(workerFactory, comlinkWrapper);
      return remote[method](...arguments);
    };
  });
  return proxy;
}
async function startWorker(workerFactory, comlinkWrapper) {
  const worker = workerFactory();
  await new Promise((resolve) => {
    const readyListener = (event) => {
      removeEventListener(worker, "message", readyListener);
      if (getEventData(event) === "NIMIQ_ONLOAD") resolve();
    };
    addEventListener(worker, "message", readyListener);
  });
  console.debug("Have crypto worker remote");
  return comlinkWrapper(worker);
}
function addEventListener(worker, type, listener) {
  const method = "addListener" in worker ? "addListener" : "addEventListener";
  worker[method](type, listener);
}
function removeEventListener(worker, type, listener) {
  const method = "removeListener" in worker ? "removeListener" : "removeEventListener";
  worker[method](type, listener);
}
function getEventData(event) {
  return typeof event === "object" && "data" in event ? event.data : event;
}
export {
  cryptoUtilsWorkerFactory
};
