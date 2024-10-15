export class ArrayUtils {
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
