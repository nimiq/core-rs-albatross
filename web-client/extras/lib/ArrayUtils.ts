/**
 * A `Uint8Array` that is backed by a plain (non-shared) `ArrayBuffer`.
 *
 * Since TypeScript 5.7, `Uint8Array` is generic over its backing buffer, and the bare `Uint8Array` type means
 * `Uint8Array<ArrayBufferLike>` (which also includes `SharedArrayBuffer`). APIs like WebCrypto's `BufferSource`
 * only accept `Uint8Array<ArrayBuffer>`.
 *
 * This alias resolves to `Uint8Array<ArrayBuffer>` on TypeScript >= 5.7 and to the plain, non-generic
 * `Uint8Array` on older TypeScript versions, so it can be used in published type declarations that need to
 * work with both. (`Uint8Array.prototype.slice` always returns an array backed by a new `ArrayBuffer`, and
 * its return type is declared accordingly in all TypeScript versions.)
 */
export type ArrayBufferUint8Array = ReturnType<Uint8Array['slice']>;

export class ArrayUtils {
    /**
     * Returns a view of `uintarr` from `begin` (inclusive) to `end` (exclusive), sharing the same underlying
     * buffer. Unlike `Uint8Array.prototype.subarray`, this always returns a plain `Uint8Array`, also for
     * subclasses of `Uint8Array`.
     */
    static subarray(uintarr: ArrayBufferUint8Array, begin?: number, end?: number): ArrayBufferUint8Array;
    static subarray(uintarr: Uint8Array, begin?: number, end?: number): Uint8Array;
    static subarray(uintarr: Uint8Array, begin = 0, end = uintarr.byteLength): Uint8Array {
        function clamp(v: number, min: number, max: number) {
            return v < min ? min : v > max ? max : v;
        }

        begin = clamp(begin, 0, uintarr.byteLength);
        end = clamp(end, 0, uintarr.byteLength);

        let len = end - begin;
        if (len < 0) len = 0;

        return new Uint8Array(uintarr.buffer, uintarr.byteOffset + begin, len);
    }
}
