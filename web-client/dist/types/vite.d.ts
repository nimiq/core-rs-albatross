import type { PluginOption } from 'vite'

export interface NimiqPluginOptions {
  /**
   * Configure worker support for WASM
   * @default true
   */
  worker?: boolean
}

/**
 * Vite plugin for Nimiq blockchain integration
 * Configures WebAssembly support and optimizations required for @nimiq/core
 */
export default function nimiq(options?: NimiqPluginOptions): PluginOption[]

export { nimiq }
