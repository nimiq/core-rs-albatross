import wasm from 'vite-plugin-wasm'

/**
 * Vite plugin for Nimiq blockchain integration
 * Configures WebAssembly support and optimizations required for @nimiq/core
 *
 * Note: This plugin does not include top-level await support. Modern browsers
 * support top-level await natively. If you need support for older browsers,
 * you can add vite-plugin-top-level-await to your plugins.
 *
 * @param {object} [options] - Plugin options
 * @param {boolean} [options.worker=true] - Configure worker support for WASM
 * @returns {import('vite').Plugin[]} Array of Vite plugins
 */
export default function nimiq({ worker = true } = {}) {
  return [
    wasm(),
    {
      name: 'vite-plugin-nimiq',
      config() {
        return {
          optimizeDeps: {
            exclude: ['@nimiq/core'],
          },
          build: {
            target: 'esnext',
            rollupOptions: {
              output: {
                format: 'es',
              },
            },
          },
          ...(worker && {
            worker: {
              format: 'es',
              plugins: () => [wasm()],
            },
          }),
        }
      },
    },
  ]
}

export { nimiq }
