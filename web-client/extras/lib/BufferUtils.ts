import { SerialBuffer } from './SerialBuffer';
import { StringUtils } from './StringUtils';

type TypedArray =
    | Int8Array
    | Uint8Array
    | Int16Array
    | Uint16Array
    | Int32Array
    | Uint32Array
    | Uint8ClampedArray
    | Float32Array
    | Float64Array;

class BufferUtils {
    static BASE64_ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
    static BASE32_ALPHABET = {
        RFC4648: 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567=',
        RFC4648_HEX: '0123456789ABCDEFGHIJKLMNOPQRSTUV=',
        NIMIQ: '0123456789ABCDEFGHJKLMNPQRSTUVXY',
    };
    static HEX_ALPHABET = '0123456789abcdef';

    static _BASE64_LOOKUP: string[] = [];
    private static _ISO_8859_15_DECODER?: TextDecoder | null;
    private static _UTF8_DECODER?: TextDecoder | null;
    private static _UTF8_ENCODER?: TextEncoder | null;

    static _codePointTextDecoder(buffer: Uint8Array): string {
        if (typeof TextDecoder === 'undefined') throw new Error('TextDecoder not supported');
        if (BufferUtils._ISO_8859_15_DECODER === null) throw new Error('TextDecoder does not support iso-8859-15');
        if (BufferUtils._ISO_8859_15_DECODER === undefined) {
            try {
                BufferUtils._ISO_8859_15_DECODER = new TextDecoder('iso-8859-15');
            } catch (e) {
                BufferUtils._ISO_8859_15_DECODER = null;
                throw new Error('TextDecoder does not support iso-8859-15');
            }
        }
        const uint8View = BufferUtils._toUint8View(buffer);
        return BufferUtils._ISO_8859_15_DECODER.decode(uint8View)
            .replace(/\u20ac/g, '\u00a4') // € => ¤
            .replace(/\u0160/g, '\u00a6') // Š => ¦
            .replace(/\u0161/g, '\u00a8') // š => ¨
            .replace(/\u017d/g, '\u00b4') // Ž => ´
            .replace(/\u017e/g, '\u00b8') // ž => ¸
            .replace(/\u0152/g, '\u00bc') // Œ => ¼
            .replace(/\u0153/g, '\u00bd') // œ => ½
            .replace(/\u0178/g, '\u00be'); // Ÿ => ¾
    }

    static _tripletToBase64(num: number): string {
        return BufferUtils._BASE64_LOOKUP[num >> 18 & 0x3F] + BufferUtils._BASE64_LOOKUP[num >> 12 & 0x3F]
            + BufferUtils._BASE64_LOOKUP[num >> 6 & 0x3F] + BufferUtils._BASE64_LOOKUP[num & 0x3F];
    }

    static _base64encodeChunk(u8: Uint8Array, start: number, end: number): string {
        let tmp: number;
        const output: string[] = [];
        for (let i = start; i < end; i += 3) {
            tmp = ((u8[i] << 16) & 0xFF0000) + ((u8[i + 1] << 8) & 0xFF00) + (u8[i + 2] & 0xFF);
            output.push(BufferUtils._tripletToBase64(tmp));
        }
        return output.join('');
    }

    static _base64fromByteArray(u8: Uint8Array): string {
        let tmp: number;
        const len = u8.length;
        const extraBytes = len % 3; // if we have 1 byte left, pad 2 bytes
        let output = '';
        const parts: string[] = [];
        const maxChunkLength = 16383; // must be multiple of 3

        // go through the array every three bytes, we'll deal with trailing stuff later
        for (let i = 0, len2 = len - extraBytes; i < len2; i += maxChunkLength) {
            parts.push(
                BufferUtils._base64encodeChunk(u8, i, (i + maxChunkLength) > len2 ? len2 : (i + maxChunkLength)),
            );
        }

        // pad the end with zeros, but make sure to not forget the extra bytes
        if (extraBytes === 1) {
            tmp = u8[len - 1];
            output += BufferUtils._BASE64_LOOKUP[tmp >> 2];
            output += BufferUtils._BASE64_LOOKUP[(tmp << 4) & 0x3F];
            output += '==';
        } else if (extraBytes === 2) {
            tmp = (u8[len - 2] << 8) + (u8[len - 1]);
            output += BufferUtils._BASE64_LOOKUP[tmp >> 10];
            output += BufferUtils._BASE64_LOOKUP[(tmp >> 4) & 0x3F];
            output += BufferUtils._BASE64_LOOKUP[(tmp << 2) & 0x3F];
            output += '=';
        }

        parts.push(output);

        return parts.join('');
    }

    static toBase64(buffer: Uint8Array): string {
        if (typeof Buffer !== 'undefined') {
            return Buffer.from(buffer).toString('base64');
        }

        if (typeof TextDecoder !== 'undefined' && BufferUtils._ISO_8859_15_DECODER !== null) {
            try {
                return btoa(BufferUtils._codePointTextDecoder(buffer));
            } catch (e) {
                // Disabled itself
            }
        }

        return BufferUtils._base64fromByteArray(BufferUtils._toUint8View(buffer));
    }

    static fromBase64(base64: string, length?: number): SerialBuffer {
        let arr: Uint8Array;
        if (typeof Buffer !== 'undefined') {
            arr = Buffer.from(base64, 'base64');
        } else {
            arr = new Uint8Array(atob(base64).split('').map(c => c.charCodeAt(0)));
        }
        if (length !== undefined && arr.length !== length) {
            throw new Error('Decoded length does not match expected length');
        }
        return new SerialBuffer(arr);
    }

    static toBase64Url(buffer: Uint8Array): string {
        return BufferUtils.toBase64(buffer).replace(/\//g, '_').replace(/\+/g, '-').replace(/=/g, '');
    }

    static fromBase64Url(base64: string, length?: number): SerialBuffer {
        return BufferUtils.fromBase64(base64.replace(/_/g, '/').replace(/-/g, '+').replace(/\./g, '='), length);
    }

    static toBase32(buf: Uint8Array, alphabet: string = BufferUtils.BASE32_ALPHABET.NIMIQ): string {
        let shift = 3, carry = 0, byte: number, symbol: number, i: number, res = '';

        for (i = 0; i < buf.length; i++) {
            byte = buf[i];
            symbol = carry | (byte >> shift);
            res += alphabet[symbol & 0x1f];

            if (shift > 5) {
                shift -= 5;
                symbol = byte >> shift;
                res += alphabet[symbol & 0x1f];
            }

            shift = 5 - shift;
            carry = byte << shift;
            shift = 8 - shift;
        }

        if (shift !== 3) {
            res += alphabet[carry & 0x1f];
        }

        while (res.length % 8 !== 0 && alphabet.length === 33) {
            res += alphabet[32];
        }

        return res;
    }

    static fromBase32(base32: string, alphabet: string = BufferUtils.BASE32_ALPHABET.NIMIQ): Uint8Array {
        const charmap: Record<string, number> = {};
        alphabet.toUpperCase().split('').forEach((c, i) => {
            if (!(c in charmap)) charmap[c] = i;
        });

        let symbol: number, shift = 8, carry = 0, buf: number[] = [];
        base32.toUpperCase().split('').forEach((char) => {
            // ignore padding
            if (alphabet.length === 33 && char === alphabet[32]) return;

            symbol = charmap[char] & 0xff;

            shift -= 5;
            if (shift > 0) {
                carry |= symbol << shift;
            } else if (shift < 0) {
                buf.push(carry | (symbol >> -shift));
                shift += 8;
                carry = (symbol << shift) & 0xff;
            } else {
                buf.push(carry | symbol);
                shift = 8;
                carry = 0;
            }
        });

        if (shift !== 8 && carry !== 0) {
            buf.push(carry);
        }

        return new Uint8Array(buf);
    }

    static toHex(buffer: Uint8Array): string {
        let hex = '';
        for (let i = 0; i < buffer.length; i++) {
            const code = buffer[i];
            hex += BufferUtils.HEX_ALPHABET[code >>> 4];
            hex += BufferUtils.HEX_ALPHABET[code & 0x0F];
        }
        return hex;
    }

    static fromHex(hex: string, length?: number): SerialBuffer {
        hex = hex.trim();
        if (!StringUtils.isHexBytes(hex, length)) throw new Error('String is not a hex string (of matching length)');
        return new SerialBuffer(new Uint8Array((hex.match(/.{2}/g) || []).map(byte => parseInt(byte, 16))));
    }

    private static _utf8TextEncoder(str: string): Uint8Array {
        if (typeof TextEncoder === 'undefined') throw new Error('TextEncoder not supported');
        if (BufferUtils._UTF8_ENCODER === null) throw new Error('TextEncoder does not support utf8');
        if (BufferUtils._UTF8_ENCODER === undefined) {
            try {
                BufferUtils._UTF8_ENCODER = new TextEncoder();
            } catch (e) {
                BufferUtils._UTF8_ENCODER = null;
                throw new Error('TextEncoder does not support utf8');
            }
        }
        return BufferUtils._UTF8_ENCODER.encode(str);
    }

    static fromUtf8(str: string): Uint8Array {
        return BufferUtils._utf8TextEncoder(str);
    }

    private static _utf8TextDecoder(buf: TypedArray, options?: TextDecoderOptions): string {
        if (typeof TextDecoder === 'undefined') throw new Error('TextDecoder not supported');
        if (BufferUtils._UTF8_DECODER === null) throw new Error('TextDecoder does not support utf8');
        if (BufferUtils._UTF8_DECODER === undefined) {
            try {
                BufferUtils._UTF8_DECODER = new TextDecoder('utf-8', options);
            } catch (e) {
                BufferUtils._UTF8_DECODER = null;
                throw new Error('TextDecoder does not support utf8');
            }
        }
        return BufferUtils._UTF8_DECODER.decode(buf);
    }

    static toUtf8(buf: TypedArray): string {
        return BufferUtils._utf8TextDecoder(buf);
    }

    static fromAny(o: Uint8Array | string, length?: number): SerialBuffer {
        if (o === '') return SerialBuffer.EMPTY;
        if (!o) throw new Error('Invalid buffer format');
        if (o instanceof Uint8Array) return new SerialBuffer(o);
        try {
            return BufferUtils.fromHex(o, length);
        } catch (e) {
            // Ignore
        }
        try {
            return BufferUtils.fromBase64(o, length);
        } catch (e) {
            // Ignore
        }
        throw new Error('Invalid buffer format');
    }

    static equals(a: TypedArray, b: TypedArray): boolean {
        const viewA = BufferUtils._toUint8View(a);
        const viewB = BufferUtils._toUint8View(b);
        if (viewA.length !== viewB.length) return false;
        for (let i = 0; i < viewA.length; i++) {
            if (viewA[i] !== viewB[i]) return false;
        }
        return true;
    }

    /**
     * Returns -1 if a is smaller than b, 1 if a is larger than b, 0 if a equals b.
     * Shorter arrays are always considered smaller than longer ones.
     */
    static compare(a: TypedArray, b: TypedArray): number {
        if (a.length < b.length) return -1;
        if (a.length > b.length) return 1;
        for (let i = 0; i < a.length; i++) {
            if (a[i] < b[i]) return -1;
            if (a[i] > b[i]) return 1;
        }
        return 0;
    }

    static xor(a: Uint8Array, b: Uint8Array): Uint8Array {
        const res = new Uint8Array(a.byteLength);
        for (let i = 0; i < a.byteLength; ++i) {
            res[i] = a[i] ^ b[i];
        }
        return res;
    }

    private static _toUint8View(arrayLike: TypedArray | ArrayBuffer): Uint8Array {
        if (arrayLike instanceof Uint8Array) {
            return arrayLike;
        } else if (arrayLike instanceof ArrayBuffer) {
            return new Uint8Array(arrayLike);
        } else if (arrayLike.buffer instanceof ArrayBuffer) {
            return new Uint8Array(arrayLike.buffer);
        } else {
            throw new Error('TypedArray or ArrayBuffer required');
        }
    }
}

for (let i = 0, len = BufferUtils.BASE64_ALPHABET.length; i < len; ++i) {
    BufferUtils._BASE64_LOOKUP[i] = BufferUtils.BASE64_ALPHABET[i];
}

export { BufferUtils };
