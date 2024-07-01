export class ArrayUtils {
    static randomElement<T>(arr: T[]): T {
        return arr[Math.floor(Math.random() * arr.length)];
    }

    static subarray(uintarr: Uint8Array, begin?: number, end?: number): Uint8Array {
        function clamp(v: number, min: number, max: number) {
            return v < min ? min : v > max ? max : v;
        }

        if (begin === undefined) begin = 0;
        if (end === undefined) end = uintarr.byteLength;

        begin = clamp(begin, 0, uintarr.byteLength);
        end = clamp(end, 0, uintarr.byteLength);

        let len = end - begin;
        if (len < 0) {
            len = 0;
        }

        return new Uint8Array(uintarr.buffer, uintarr.byteOffset + begin, len);
    }

    static *k_combinations(list: any[], k: number): Generator<any[]> {
        const n = list.length;
        // Shortcut:
        if (k > n) {
            return;
        }
        const indices = Array.from(new Array(k), (x, i) => i);
        yield indices.map(i => list[i]);
        const reverseRange = Array.from(new Array(k), (x, i) => k - i - 1);
        /*eslint no-constant-condition: ["error", { "checkLoops": false }]*/
        while (true) {
            let i = k - 1, found = false;
            for (i of reverseRange) {
                if (indices[i] !== i + n - k) {
                    found = true;
                    break;
                }
            }
            if (!found) {
                return;
            }
            indices[i] += 1;
            for (const j of Array.from(new Array(k - i - 1), (x, k) => i + k + 1)) {
                indices[j] = indices[j - 1] + 1;
            }
            yield indices.map(i => list[i]);
        }
    }
}
