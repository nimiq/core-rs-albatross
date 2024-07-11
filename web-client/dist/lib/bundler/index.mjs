// lib/ArrayUtils.ts
var ArrayUtils = class {
  static randomElement(arr) {
    return arr[Math.floor(Math.random() * arr.length)];
  }
  static subarray(uintarr, begin, end) {
    function clamp(v, min, max) {
      return v < min ? min : v > max ? max : v;
    }
    if (begin === void 0) begin = 0;
    if (end === void 0) end = uintarr.byteLength;
    begin = clamp(begin, 0, uintarr.byteLength);
    end = clamp(end, 0, uintarr.byteLength);
    let len = end - begin;
    if (len < 0) {
      len = 0;
    }
    return new Uint8Array(uintarr.buffer, uintarr.byteOffset + begin, len);
  }
  static *k_combinations(list, k) {
    const n = list.length;
    if (k > n) {
      return;
    }
    const indices = Array.from(new Array(k), (x, i) => i);
    yield indices.map((i) => list[i]);
    const reverseRange = Array.from(new Array(k), (x, i) => k - i - 1);
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
      for (const j of Array.from(new Array(k - i - 1), (x, k2) => i + k2 + 1)) {
        indices[j] = indices[j - 1] + 1;
      }
      yield indices.map((i2) => list[i2]);
    }
  }
};

// lib/NumberUtils.ts
var NumberUtils = class _NumberUtils {
  static UINT8_MAX = 255;
  static UINT16_MAX = 65535;
  static UINT32_MAX = 4294967295;
  static UINT64_MAX = Number.MAX_SAFE_INTEGER;
  static isInteger(val) {
    return Number.isInteger(val);
  }
  static isUint8(val) {
    return _NumberUtils.isInteger(val) && val >= 0 && val <= _NumberUtils.UINT8_MAX;
  }
  static isUint16(val) {
    return _NumberUtils.isInteger(val) && val >= 0 && val <= _NumberUtils.UINT16_MAX;
  }
  static isUint32(val) {
    return _NumberUtils.isInteger(val) && val >= 0 && val <= _NumberUtils.UINT32_MAX;
  }
  static isUint64(val) {
    return _NumberUtils.isInteger(val) && val >= 0 && val <= _NumberUtils.UINT64_MAX;
  }
  static randomUint32() {
    return Math.floor(Math.random() * (_NumberUtils.UINT32_MAX + 1));
  }
  static randomUint64() {
    return Math.floor(Math.random() * (_NumberUtils.UINT64_MAX + 1));
  }
  static fromBinary(bin) {
    return parseInt(bin, 2);
  }
};

// lib/StringUtils.ts
var StringUtils = class _StringUtils {
  static isMultibyte(str) {
    return /[\uD800-\uDFFF]/.test(str);
  }
  static isHex(str) {
    return /^[0-9A-Fa-f]*$/.test(str);
  }
  static isHexBytes(str, length) {
    if (!_StringUtils.isHex(str)) return false;
    if (str.length % 2 !== 0) return false;
    if (typeof length === "number" && str.length / 2 !== length) return false;
    return true;
  }
  static commonPrefix(str1, str2) {
    let i = 0;
    for (; i < str1.length; ++i) {
      if (str1[i] !== str2[i]) break;
    }
    return str1.substring(0, i);
  }
  static lpad(str, padString, length) {
    while (str.length < length) str = padString + str;
    return str;
  }
};

// lib/SerialBuffer.ts
var SerialBuffer = class _SerialBuffer extends Uint8Array {
  _view;
  _readPos;
  _writePos;
  static EMPTY = new _SerialBuffer(0);
  constructor(bufferOrArrayOrLength) {
    super(bufferOrArrayOrLength);
    this._view = new DataView(this.buffer);
    this._readPos = 0;
    this._writePos = 0;
  }
  subarray(start, end) {
    return ArrayUtils.subarray(this, start, end);
  }
  get readPos() {
    return this._readPos;
  }
  set readPos(value) {
    if (value < 0 || value > this.byteLength) throw `Invalid readPos ${value}`;
    this._readPos = value;
  }
  get writePos() {
    return this._writePos;
  }
  set writePos(value) {
    if (value < 0 || value > this.byteLength) throw `Invalid writePos ${value}`;
    this._writePos = value;
  }
  /**
   * Resets the read and write position of the buffer to zero.
   */
  reset() {
    this._readPos = 0;
    this._writePos = 0;
  }
  read(length) {
    const value = this.subarray(this._readPos, this._readPos + length);
    this._readPos += length;
    return new Uint8Array(value);
  }
  write(array) {
    this.set(array, this._writePos);
    this._writePos += array.byteLength;
  }
  readUint8() {
    return this._view.getUint8(this._readPos++);
  }
  writeUint8(value) {
    this._view.setUint8(this._writePos++, value);
  }
  readUint16() {
    const value = this._view.getUint16(this._readPos);
    this._readPos += 2;
    return value;
  }
  writeUint16(value) {
    this._view.setUint16(this._writePos, value);
    this._writePos += 2;
  }
  readUint32() {
    const value = this._view.getUint32(this._readPos);
    this._readPos += 4;
    return value;
  }
  writeUint32(value) {
    this._view.setUint32(this._writePos, value);
    this._writePos += 4;
  }
  readUint64() {
    const value = this._view.getUint32(this._readPos) * Math.pow(2, 32) + this._view.getUint32(this._readPos + 4);
    if (!NumberUtils.isUint64(value)) throw new Error("Malformed value");
    this._readPos += 8;
    return value;
  }
  writeUint64(value) {
    if (!NumberUtils.isUint64(value)) throw new Error("Malformed value");
    this._view.setUint32(this._writePos, Math.floor(value / Math.pow(2, 32)));
    this._view.setUint32(this._writePos + 4, value);
    this._writePos += 8;
  }
  readVarUint() {
    const value = this.readUint8();
    if (value < 253) {
      return value;
    } else if (value === 253) {
      return this.readUint16();
    } else if (value === 254) {
      return this.readUint32();
    } else {
      return this.readUint64();
    }
  }
  writeVarUint(value) {
    if (!NumberUtils.isUint64(value)) throw new Error("Malformed value");
    if (value < 253) {
      this.writeUint8(value);
    } else if (value <= 65535) {
      this.writeUint8(253);
      this.writeUint16(value);
    } else if (value <= 4294967295) {
      this.writeUint8(254);
      this.writeUint32(value);
    } else {
      this.writeUint8(255);
      this.writeUint64(value);
    }
  }
  static varUintSize(value) {
    if (!NumberUtils.isUint64(value)) throw new Error("Malformed value");
    if (value < 253) {
      return 1;
    } else if (value <= 65535) {
      return 3;
    } else if (value <= 4294967295) {
      return 5;
    } else {
      return 9;
    }
  }
  readFloat64() {
    const value = this._view.getFloat64(this._readPos);
    this._readPos += 8;
    return value;
  }
  writeFloat64(value) {
    this._view.setFloat64(this._writePos, value);
    this._writePos += 8;
  }
  readString(length) {
    const bytes = this.read(length);
    return BufferUtils.toAscii(bytes);
  }
  writeString(value, length) {
    if (StringUtils.isMultibyte(value) || value.length !== length) throw new Error("Malformed value/length");
    const bytes = BufferUtils.fromAscii(value);
    this.write(bytes);
  }
  readPaddedString(length) {
    const bytes = this.read(length);
    let i = 0;
    while (i < length && bytes[i] !== 0) i++;
    const view = new Uint8Array(bytes.buffer, bytes.byteOffset, i);
    return BufferUtils.toAscii(view);
  }
  writePaddedString(value, length) {
    if (StringUtils.isMultibyte(value) || value.length > length) throw new Error("Malformed value/length");
    const bytes = BufferUtils.fromAscii(value);
    this.write(bytes);
    const padding = length - bytes.byteLength;
    this.write(new Uint8Array(padding));
  }
  readVarLengthString() {
    const length = this.readUint8();
    if (this._readPos + length > this.length) throw new Error("Malformed length");
    const bytes = this.read(length);
    return BufferUtils.toAscii(bytes);
  }
  writeVarLengthString(value) {
    if (StringUtils.isMultibyte(value) || !NumberUtils.isUint8(value.length)) throw new Error("Malformed value");
    const bytes = BufferUtils.fromAscii(value);
    this.writeUint8(bytes.byteLength);
    this.write(bytes);
  }
  static varLengthStringSize(value) {
    if (StringUtils.isMultibyte(value) || !NumberUtils.isUint8(value.length)) throw new Error("Malformed value");
    return (
      /*length*/
      1 + value.length
    );
  }
};

// lib/BufferUtils.ts
var BufferUtils = class _BufferUtils {
  static BASE64_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  static BASE32_ALPHABET = {
    RFC4648: "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567=",
    RFC4648_HEX: "0123456789ABCDEFGHIJKLMNOPQRSTUV=",
    NIMIQ: "0123456789ABCDEFGHJKLMNPQRSTUVXY"
  };
  static HEX_ALPHABET = "0123456789abcdef";
  static _BASE64_LOOKUP = [];
  static _ISO_8859_15_DECODER;
  static _UTF8_ENCODER;
  static toAscii(buffer) {
    const chunkSize = 8192;
    const buf = _BufferUtils._toUint8View(buffer);
    let ascii = "";
    for (let i = 0; i < buf.length; i += chunkSize) {
      ascii += String.fromCharCode.apply(null, [...buf.subarray(i, i + chunkSize)]);
    }
    return ascii;
  }
  static fromAscii(string) {
    const buf = new Uint8Array(string.length);
    for (let i = 0; i < string.length; ++i) {
      buf[i] = string.charCodeAt(i);
    }
    return buf;
  }
  static _codePointTextDecoder(buffer) {
    if (typeof TextDecoder === "undefined") throw new Error("TextDecoder not supported");
    if (_BufferUtils._ISO_8859_15_DECODER === null) throw new Error("TextDecoder does not support iso-8859-15");
    if (_BufferUtils._ISO_8859_15_DECODER === void 0) {
      try {
        _BufferUtils._ISO_8859_15_DECODER = new TextDecoder("iso-8859-15");
      } catch (e) {
        _BufferUtils._ISO_8859_15_DECODER = null;
        throw new Error("TextDecoder does not support iso-8859-15");
      }
    }
    const uint8View = _BufferUtils._toUint8View(buffer);
    return _BufferUtils._ISO_8859_15_DECODER.decode(uint8View).replace(/\u20ac/g, "\xA4").replace(/\u0160/g, "\xA6").replace(/\u0161/g, "\xA8").replace(/\u017d/g, "\xB4").replace(/\u017e/g, "\xB8").replace(/\u0152/g, "\xBC").replace(/\u0153/g, "\xBD").replace(/\u0178/g, "\xBE");
  }
  static _tripletToBase64(num) {
    return _BufferUtils._BASE64_LOOKUP[num >> 18 & 63] + _BufferUtils._BASE64_LOOKUP[num >> 12 & 63] + _BufferUtils._BASE64_LOOKUP[num >> 6 & 63] + _BufferUtils._BASE64_LOOKUP[num & 63];
  }
  static _base64encodeChunk(u8, start, end) {
    let tmp;
    const output = [];
    for (let i = start; i < end; i += 3) {
      tmp = (u8[i] << 16 & 16711680) + (u8[i + 1] << 8 & 65280) + (u8[i + 2] & 255);
      output.push(_BufferUtils._tripletToBase64(tmp));
    }
    return output.join("");
  }
  static _base64fromByteArray(u8) {
    let tmp;
    const len = u8.length;
    const extraBytes = len % 3;
    let output = "";
    const parts = [];
    const maxChunkLength = 16383;
    for (let i = 0, len2 = len - extraBytes; i < len2; i += maxChunkLength) {
      parts.push(
        _BufferUtils._base64encodeChunk(u8, i, i + maxChunkLength > len2 ? len2 : i + maxChunkLength)
      );
    }
    if (extraBytes === 1) {
      tmp = u8[len - 1];
      output += _BufferUtils._BASE64_LOOKUP[tmp >> 2];
      output += _BufferUtils._BASE64_LOOKUP[tmp << 4 & 63];
      output += "==";
    } else if (extraBytes === 2) {
      tmp = (u8[len - 2] << 8) + u8[len - 1];
      output += _BufferUtils._BASE64_LOOKUP[tmp >> 10];
      output += _BufferUtils._BASE64_LOOKUP[tmp >> 4 & 63];
      output += _BufferUtils._BASE64_LOOKUP[tmp << 2 & 63];
      output += "=";
    }
    parts.push(output);
    return parts.join("");
  }
  static toBase64(buffer) {
    if (typeof TextDecoder !== "undefined" && _BufferUtils._ISO_8859_15_DECODER !== null) {
      try {
        return btoa(_BufferUtils._codePointTextDecoder(buffer));
      } catch (e) {
      }
    }
    return _BufferUtils._base64fromByteArray(_BufferUtils._toUint8View(buffer));
  }
  static fromBase64(base64, length) {
    const arr = new Uint8Array(atob(base64).split("").map((c) => c.charCodeAt(0)));
    if (length !== void 0 && arr.length !== length) {
      throw new Error("Decoded length does not match expected length");
    }
    return new SerialBuffer(arr);
  }
  static toBase64Url(buffer) {
    return _BufferUtils.toBase64(buffer).replace(/\//g, "_").replace(/\+/g, "-").replace(/=/g, ".");
  }
  static fromBase64Url(base64, length) {
    return _BufferUtils.fromBase64(base64.replace(/_/g, "/").replace(/-/g, "+").replace(/\./g, "="), length);
  }
  static toBase32(buf, alphabet = _BufferUtils.BASE32_ALPHABET.NIMIQ) {
    let shift = 3, carry = 0, byte, symbol, i, res = "";
    for (i = 0; i < buf.length; i++) {
      byte = buf[i];
      symbol = carry | byte >> shift;
      res += alphabet[symbol & 31];
      if (shift > 5) {
        shift -= 5;
        symbol = byte >> shift;
        res += alphabet[symbol & 31];
      }
      shift = 5 - shift;
      carry = byte << shift;
      shift = 8 - shift;
    }
    if (shift !== 3) {
      res += alphabet[carry & 31];
    }
    while (res.length % 8 !== 0 && alphabet.length === 33) {
      res += alphabet[32];
    }
    return res;
  }
  static fromBase32(base32, alphabet = _BufferUtils.BASE32_ALPHABET.NIMIQ) {
    const charmap = {};
    alphabet.toUpperCase().split("").forEach((c, i) => {
      if (!(c in charmap)) charmap[c] = i;
    });
    let symbol, shift = 8, carry = 0, buf = [];
    base32.toUpperCase().split("").forEach((char) => {
      if (alphabet.length === 33 && char === alphabet[32]) return;
      symbol = charmap[char] & 255;
      shift -= 5;
      if (shift > 0) {
        carry |= symbol << shift;
      } else if (shift < 0) {
        buf.push(carry | symbol >> -shift);
        shift += 8;
        carry = symbol << shift & 255;
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
  static toHex(buffer) {
    let hex = "";
    for (let i = 0; i < buffer.length; i++) {
      const code = buffer[i];
      hex += _BufferUtils.HEX_ALPHABET[code >>> 4];
      hex += _BufferUtils.HEX_ALPHABET[code & 15];
    }
    return hex;
  }
  static fromHex(hex, length) {
    hex = hex.trim();
    if (!StringUtils.isHexBytes(hex, length)) throw new Error("String is not an hex string (of matching length)");
    return new SerialBuffer(new Uint8Array((hex.match(/.{2}/g) || []).map((byte) => parseInt(byte, 16))));
  }
  static toBinary(buffer) {
    let bin = "";
    for (let i = 0; i < buffer.length; i++) {
      const code = buffer[i];
      bin += StringUtils.lpad(code.toString(2), "0", 8);
    }
    return bin;
  }
  static _strToUint8Array(str) {
    const out = [];
    let p = 0;
    for (let i = 0; i < str.length; i++) {
      let c = str.charCodeAt(i);
      if (c < 128) {
        out[p++] = c;
      } else if (c < 2048) {
        out[p++] = c >> 6 | 192;
        out[p++] = c & 63 | 128;
      } else if ((c & 64512) === 55296 && i + 1 < str.length && (str.charCodeAt(i + 1) & 64512) === 56320) {
        c = 65536 + ((c & 1023) << 10) + (str.charCodeAt(++i) & 1023);
        out[p++] = c >> 18 | 240;
        out[p++] = c >> 12 & 63 | 128;
        out[p++] = c >> 6 & 63 | 128;
        out[p++] = c & 63 | 128;
      } else {
        out[p++] = c >> 12 | 224;
        out[p++] = c >> 6 & 63 | 128;
        out[p++] = c & 63 | 128;
      }
    }
    return new Uint8Array(out);
  }
  static _utf8TextEncoder(str) {
    if (typeof TextEncoder === "undefined") throw new Error("TextEncoder not supported");
    if (_BufferUtils._UTF8_ENCODER === null) throw new Error("TextEncoder does not support utf8");
    if (_BufferUtils._UTF8_ENCODER === void 0) {
      try {
        _BufferUtils._UTF8_ENCODER = new TextEncoder();
      } catch (e) {
        _BufferUtils._UTF8_ENCODER = null;
        throw new Error("TextEncoder does not support utf8");
      }
    }
    return _BufferUtils._UTF8_ENCODER.encode(str);
  }
  static fromUtf8(str) {
    if (typeof TextEncoder !== "undefined" && _BufferUtils._UTF8_ENCODER !== null) {
      try {
        return _BufferUtils._utf8TextEncoder(str);
      } catch (e) {
      }
    }
    return _BufferUtils._strToUint8Array(str);
  }
  static fromAny(o, length) {
    if (o === "") return SerialBuffer.EMPTY;
    if (!o) throw new Error("Invalid buffer format");
    if (o instanceof Uint8Array) return new SerialBuffer(o);
    try {
      return _BufferUtils.fromHex(o, length);
    } catch (e) {
    }
    try {
      return _BufferUtils.fromBase64(o, length);
    } catch (e) {
    }
    throw new Error("Invalid buffer format");
  }
  static concatTypedArrays(a, b) {
    const c = new a.constructor(a.length + b.length);
    c.set(a, 0);
    c.set(b, a.length);
    return c;
  }
  static equals(a, b) {
    const viewA = _BufferUtils._toUint8View(a);
    const viewB = _BufferUtils._toUint8View(b);
    if (viewA.length !== viewB.length) return false;
    for (let i = 0; i < viewA.length; i++) {
      if (viewA[i] !== viewB[i]) return false;
    }
    return true;
  }
  /**
   * Returns -1 if a is smaller than b, 1 if a is larger than b, 0 if a equals b.
   */
  static compare(a, b) {
    if (a.length < b.length) return -1;
    if (a.length > b.length) return 1;
    for (let i = 0; i < a.length; i++) {
      if (a[i] < b[i]) return -1;
      if (a[i] > b[i]) return 1;
    }
    return 0;
  }
  static xor(a, b) {
    const res = new Uint8Array(a.byteLength);
    for (let i = 0; i < a.byteLength; ++i) {
      res[i] = a[i] ^ b[i];
    }
    return res;
  }
  static _toUint8View(arrayLike) {
    if (arrayLike instanceof Uint8Array) {
      return arrayLike;
    } else if (arrayLike instanceof ArrayBuffer) {
      return new Uint8Array(arrayLike);
    } else if (arrayLike.buffer instanceof ArrayBuffer) {
      return new Uint8Array(arrayLike.buffer);
    } else {
      throw new Error("TypedArray or ArrayBuffer required");
    }
  }
};
for (let i = 0, len = BufferUtils.BASE64_ALPHABET.length; i < len; ++i) {
  BufferUtils._BASE64_LOOKUP[i] = BufferUtils.BASE64_ALPHABET[i];
}

// lib/Entropy.ts
import { CryptoUtils as CryptoUtils4 } from "../../bundler/index.js";

// lib/ExtendedPrivateKey.ts
import { CryptoUtils, PrivateKey, PublicKey } from "../../bundler/index.js";

// lib/Serializable.ts
var Serializable = class _Serializable {
  /**
   * Checks for equality with another Serializable.
   */
  equals(o) {
    return o instanceof _Serializable && BufferUtils.equals(this.serialize(), o.serialize());
  }
  /**
   * Compares this object to another object.
   *
   * Returns a negative number if `this` is smaller than o, a positive number if `this` is larger than o, and zero if equal.
   */
  compare(o) {
    return BufferUtils.compare(this.serialize(), o.serialize());
  }
  /**
   * Formats the object into a hex string.
   */
  toString() {
    return this.toHex();
  }
  /**
   * Formats the object into a base64 string.
   */
  toBase64() {
    return BufferUtils.toBase64(this.serialize());
  }
  /**
   * Formats the object into a hex string.
   */
  toHex() {
    return BufferUtils.toHex(this.serialize());
  }
};

// lib/ExtendedPrivateKey.ts
var ExtendedPrivateKey = class _ExtendedPrivateKey extends Serializable {
  static CHAIN_CODE_SIZE = 32;
  _key;
  _chainCode;
  /**
   * Creates an ExtendedPrivateKey from a private key and chain code.
   */
  constructor(key, chainCode) {
    super();
    if (!(key instanceof PrivateKey)) throw new Error("ExtendedPrivateKey: Invalid key");
    if (!(chainCode instanceof Uint8Array)) throw new Error("ExtendedPrivateKey: Invalid chainCode");
    if (chainCode.length !== _ExtendedPrivateKey.CHAIN_CODE_SIZE) {
      throw new Error("ExtendedPrivateKey: Invalid chainCode length");
    }
    this._key = key;
    this._chainCode = chainCode;
  }
  /**
   * Generates the master ExtendedPrivateKey from a seed.
   */
  static generateMasterKey(seed) {
    const bCurve = BufferUtils.fromAscii("ed25519 seed");
    const hash = CryptoUtils.computeHmacSha512(bCurve, seed);
    return new _ExtendedPrivateKey(new PrivateKey(hash.slice(0, 32)), hash.slice(32));
  }
  /**
   * Derives a child ExtendedPrivateKey from the current key at the provided index.
   */
  derive(index) {
    if (index < 2147483648) index += 2147483648;
    const data = new SerialBuffer(1 + PrivateKey.SIZE + 4);
    data.writeUint8(0);
    data.write(this._key.serialize());
    data.writeUint32(index);
    const hash = CryptoUtils.computeHmacSha512(this._chainCode, data);
    return new _ExtendedPrivateKey(new PrivateKey(hash.slice(0, 32)), hash.slice(32));
  }
  /**
   * Tests if a HD derivation path is valid.
   */
  static isValidPath(path) {
    if (path.match(/^m(\/[0-9]+')*$/) === null) return false;
    const segments = path.split("/");
    for (let i = 1; i < segments.length; i++) {
      if (!NumberUtils.isUint32(parseInt(segments[i]))) return false;
    }
    return true;
  }
  /**
   * Derives a child ExtendedPrivateKey from the current key at the provided path.
   */
  derivePath(path) {
    if (!_ExtendedPrivateKey.isValidPath(path)) throw new Error("Invalid path");
    let extendedKey = this;
    const segments = path.split("/");
    for (let i = 1; i < segments.length; i++) {
      const index = parseInt(segments[i]);
      extendedKey = extendedKey.derive(index);
    }
    return extendedKey;
  }
  /**
   * Derives an ExtendedPrivateKey from a seed and a derivation path.
   */
  static derivePathFromSeed(path, seed) {
    let extendedKey = _ExtendedPrivateKey.generateMasterKey(seed);
    return extendedKey.derivePath(path);
  }
  /**
   * Deserializes an ExtendedPrivateKey from a byte array.
   */
  static unserialize(buf) {
    const privateKey = PrivateKey.unserialize(buf);
    const chainCode = buf.read(_ExtendedPrivateKey.CHAIN_CODE_SIZE);
    return new _ExtendedPrivateKey(privateKey, chainCode);
  }
  /**
   * Deserializes an ExtendedPrivateKey from a hex string.
   */
  static fromHex(hex) {
    return _ExtendedPrivateKey.unserialize(BufferUtils.fromHex(hex));
  }
  /**
   * Serializes the ExtendedPrivateKey to a byte array.
   */
  serialize(buf) {
    buf = buf || new SerialBuffer(this.serializedSize);
    buf.write(this._key.serialize());
    buf.write(this._chainCode);
    return buf;
  }
  /**
   * Returns the serialized size of this ExtendedPrivateKey.
   */
  get serializedSize() {
    return this._key.serializedSize + _ExtendedPrivateKey.CHAIN_CODE_SIZE;
  }
  /**
   * Checks for equality with another ExtendedPrivateKey.
   */
  equals(o) {
    return o instanceof _ExtendedPrivateKey && super.equals(o);
  }
  /**
   * Returns the private key of this ExtendedPrivateKey.
   */
  get privateKey() {
    return this._key;
  }
  /**
   * Returns the address related to this ExtendedPrivateKey.
   */
  toAddress() {
    return PublicKey.derive(this._key).toAddress();
  }
};

// lib/MnemonicUtils.ts
import { CryptoUtils as CryptoUtils2, Hash } from "../../bundler/index.js";

// lib/CRC8.ts
var CRC8 = class _CRC8 {
  static _table = null;
  // Adapted from https://github.com/mode80/crc8js
  static _createTable() {
    const table = [];
    for (let i = 0; i < 256; ++i) {
      let curr = i;
      for (let j = 0; j < 8; ++j) {
        if ((curr & 128) !== 0) {
          curr = (curr << 1 ^ 151) % 256;
        } else {
          curr = (curr << 1) % 256;
        }
      }
      table[i] = curr;
    }
    return table;
  }
  static compute(buf) {
    if (!_CRC8._table) _CRC8._table = _CRC8._createTable();
    let c = 0;
    for (let i = 0; i < buf.length; i++) {
      c = _CRC8._table[(c ^ buf[i]) % 256];
    }
    return c;
  }
};

// lib/MnemonicUtils.ts
var MnemonicUtils = class _MnemonicUtils {
  // Adapted from https://github.com/bitcoinjs/bip39, see license below.
  /**
   * The English wordlist.
   */
  static ENGLISH_WORDLIST;
  /**
   * The default English wordlist.
   */
  static DEFAULT_WORDLIST;
  /**
   * Converts an Entropy to a mnemonic.
   */
  static entropyToMnemonic(entropy, wordlist) {
    wordlist = wordlist || _MnemonicUtils.DEFAULT_WORDLIST;
    const normalized = _MnemonicUtils._normalizeEntropy(entropy);
    const entropyBits = _MnemonicUtils._entropyToBits(normalized);
    const checksumBits = _MnemonicUtils._sha256Checksum(normalized);
    const bits = entropyBits + checksumBits;
    return _MnemonicUtils._bitsToMnemonic(bits, wordlist);
  }
  /**
   * Converts a mnemonic to an Entropy.
   */
  static mnemonicToEntropy(mnemonic, wordlist) {
    if (!Array.isArray(mnemonic)) mnemonic = mnemonic.trim().split(/\s+/g);
    wordlist = wordlist || _MnemonicUtils.DEFAULT_WORDLIST;
    const bits = _MnemonicUtils._mnemonicToBits(mnemonic, wordlist);
    return new Entropy(_MnemonicUtils._bitsToEntropy(bits, false));
  }
  /**
   * Converts a mnemonic to a seed.
   *
   * Optionally takes a password to use for the seed derivation.
   */
  static mnemonicToSeed(mnemonic, password) {
    if (Array.isArray(mnemonic)) mnemonic = mnemonic.join(" ");
    const mnemonicBuffer = BufferUtils.fromAscii(mnemonic);
    const saltBuffer = BufferUtils.fromAscii(_MnemonicUtils._salt(password));
    return new SerialBuffer(CryptoUtils2.computePBKDF2sha512(mnemonicBuffer, saltBuffer, 2048, 64));
  }
  /**
   * Converts a mnemonic to an extended private key.
   *
   * Optionally takes a password to use for the seed derivation.
   */
  static mnemonicToExtendedPrivateKey(mnemonic, password) {
    const seed = _MnemonicUtils.mnemonicToSeed(mnemonic, password);
    return ExtendedPrivateKey.generateMasterKey(seed);
  }
  /**
   * Tests if a mnemonic can be both for a legacy Nimiq wallet and a BIP39 wallet.
   */
  static isCollidingChecksum(entropy) {
    const normalizedEntropy = _MnemonicUtils._normalizeEntropy(entropy);
    return _MnemonicUtils._crcChecksum(normalizedEntropy) === _MnemonicUtils._sha256Checksum(normalizedEntropy);
  }
  /**
   * Gets the type of a mnemonic.
   *
   * Return values:
   * - `0 = MnemonicType.LEGACY`: the mnemonic is for a legacy Nimiq wallet.
   * - `1 = MnemonicType.BIP39`: the mnemonic is for a BIP39 wallet.
   * - `-1 = MnemonicType.UNKNOWN`: the mnemonic can be for both.
   *
   * Throws if the menmonic is invalid.
   */
  static getMnemonicType(mnemonic, wordlist) {
    if (!Array.isArray(mnemonic)) mnemonic = mnemonic.trim().split(/\s+/g);
    wordlist = wordlist || _MnemonicUtils.DEFAULT_WORDLIST;
    const bits = _MnemonicUtils._mnemonicToBits(mnemonic, wordlist);
    let isBIP39 = true;
    try {
      _MnemonicUtils._bitsToEntropy(bits, false);
    } catch (e) {
      isBIP39 = false;
    }
    let isLegacy = true;
    try {
      _MnemonicUtils._bitsToEntropy(bits, true);
    } catch (e) {
      isLegacy = false;
    }
    if (isBIP39 && isLegacy) return _MnemonicUtils.MnemonicType.UNKNOWN;
    if (!isBIP39 && !isLegacy) throw new Error("Invalid checksum");
    return isBIP39 ? _MnemonicUtils.MnemonicType.BIP39 : _MnemonicUtils.MnemonicType.LEGACY;
  }
  static _crcChecksum(entropy) {
    const ENT = entropy.length * 8;
    const CS = ENT / 32;
    const hash = CRC8.compute(entropy);
    return BufferUtils.toBinary([hash]).slice(0, CS);
  }
  static _sha256Checksum(entropy) {
    const ENT = entropy.length * 8;
    const CS = ENT / 32;
    const hash = Hash.computeSha256(entropy);
    return BufferUtils.toBinary(hash).slice(0, CS);
  }
  static _entropyToBits(entropy) {
    if (entropy.length < 16) throw new Error("Invalid key, length < 16");
    if (entropy.length > 32) throw new Error("Invalid key, length > 32");
    if (entropy.length % 4 !== 0) throw new Error("Invalid key, length % 4 != 0");
    return BufferUtils.toBinary(entropy);
  }
  static _normalizeEntropy(entropy) {
    let normalized;
    if (typeof entropy === "string") normalized = BufferUtils.fromHex(entropy);
    else if (entropy instanceof Entropy) normalized = entropy.serialize();
    else if (entropy instanceof ArrayBuffer) normalized = new Uint8Array(entropy);
    else normalized = entropy;
    return normalized;
  }
  static _bitsToMnemonic(bits, wordlist) {
    const chunks = bits.match(/(.{11})/g);
    if (!chunks) throw new Error("Invalid bits, less than 11 characters");
    const words = chunks.map((chunk) => {
      const index = NumberUtils.fromBinary(chunk);
      return wordlist[index];
    });
    return words;
  }
  static _mnemonicToBits(mnemonic, wordlist) {
    const words = mnemonic;
    if (words.length < 12) throw new Error("Invalid mnemonic, less than 12 words");
    if (words.length > 24) throw new Error("Invalid mnemonic, more than 24 words");
    if (words.length % 3 !== 0) throw new Error("Invalid mnemonic, words % 3 != 0");
    const bits = words.map(function(word) {
      const index = wordlist.indexOf(word.toLowerCase());
      if (index === -1) throw new Error(`Invalid mnemonic, word >${word}< is not in wordlist`);
      return StringUtils.lpad(index.toString(2), "0", 11);
    }).join("");
    return bits;
  }
  static _bitsToEntropy(bits, legacy = false) {
    const dividerIndex = bits.length - (bits.length % 8 || 8);
    const entropyBits = bits.slice(0, dividerIndex);
    const checksumBits = bits.slice(dividerIndex);
    const chunks = entropyBits.match(/(.{8})/g);
    if (!chunks) throw new Error("Invalid entropyBits, less than 8 characters");
    const entropyBytes = chunks.map(NumberUtils.fromBinary);
    if (entropyBytes.length < 16) throw new Error("Invalid generated key, length < 16");
    if (entropyBytes.length > 32) throw new Error("Invalid generated key, length > 32");
    if (entropyBytes.length % 4 !== 0) throw new Error("Invalid generated key, length % 4 != 0");
    const entropy = new Uint8Array(entropyBytes);
    const checksum = legacy ? _MnemonicUtils._crcChecksum(entropy) : _MnemonicUtils._sha256Checksum(entropy);
    if (checksum !== checksumBits) throw new Error("Invalid checksum");
    return entropy;
  }
  static _salt(password) {
    return `mnemonic${password || ""}`;
  }
};
MnemonicUtils.ENGLISH_WORDLIST = /* dprint-ignore */
["abandon", "ability", "able", "about", "above", "absent", "absorb", "abstract", "absurd", "abuse", "access", "accident", "account", "accuse", "achieve", "acid", "acoustic", "acquire", "across", "act", "action", "actor", "actress", "actual", "adapt", "add", "addict", "address", "adjust", "admit", "adult", "advance", "advice", "aerobic", "affair", "afford", "afraid", "again", "age", "agent", "agree", "ahead", "aim", "air", "airport", "aisle", "alarm", "album", "alcohol", "alert", "alien", "all", "alley", "allow", "almost", "alone", "alpha", "already", "also", "alter", "always", "amateur", "amazing", "among", "amount", "amused", "analyst", "anchor", "ancient", "anger", "angle", "angry", "animal", "ankle", "announce", "annual", "another", "answer", "antenna", "antique", "anxiety", "any", "apart", "apology", "appear", "apple", "approve", "april", "arch", "arctic", "area", "arena", "argue", "arm", "armed", "armor", "army", "around", "arrange", "arrest", "arrive", "arrow", "art", "artefact", "artist", "artwork", "ask", "aspect", "assault", "asset", "assist", "assume", "asthma", "athlete", "atom", "attack", "attend", "attitude", "attract", "auction", "audit", "august", "aunt", "author", "auto", "autumn", "average", "avocado", "avoid", "awake", "aware", "away", "awesome", "awful", "awkward", "axis", "baby", "bachelor", "bacon", "badge", "bag", "balance", "balcony", "ball", "bamboo", "banana", "banner", "bar", "barely", "bargain", "barrel", "base", "basic", "basket", "battle", "beach", "bean", "beauty", "because", "become", "beef", "before", "begin", "behave", "behind", "believe", "below", "belt", "bench", "benefit", "best", "betray", "better", "between", "beyond", "bicycle", "bid", "bike", "bind", "biology", "bird", "birth", "bitter", "black", "blade", "blame", "blanket", "blast", "bleak", "bless", "blind", "blood", "blossom", "blouse", "blue", "blur", "blush", "board", "boat", "body", "boil", "bomb", "bone", "bonus", "book", "boost", "border", "boring", "borrow", "boss", "bottom", "bounce", "box", "boy", "bracket", "brain", "brand", "brass", "brave", "bread", "breeze", "brick", "bridge", "brief", "bright", "bring", "brisk", "broccoli", "broken", "bronze", "broom", "brother", "brown", "brush", "bubble", "buddy", "budget", "buffalo", "build", "bulb", "bulk", "bullet", "bundle", "bunker", "burden", "burger", "burst", "bus", "business", "busy", "butter", "buyer", "buzz", "cabbage", "cabin", "cable", "cactus", "cage", "cake", "call", "calm", "camera", "camp", "can", "canal", "cancel", "candy", "cannon", "canoe", "canvas", "canyon", "capable", "capital", "captain", "car", "carbon", "card", "cargo", "carpet", "carry", "cart", "case", "cash", "casino", "castle", "casual", "cat", "catalog", "catch", "category", "cattle", "caught", "cause", "caution", "cave", "ceiling", "celery", "cement", "census", "century", "cereal", "certain", "chair", "chalk", "champion", "change", "chaos", "chapter", "charge", "chase", "chat", "cheap", "check", "cheese", "chef", "cherry", "chest", "chicken", "chief", "child", "chimney", "choice", "choose", "chronic", "chuckle", "chunk", "churn", "cigar", "cinnamon", "circle", "citizen", "city", "civil", "claim", "clap", "clarify", "claw", "clay", "clean", "clerk", "clever", "click", "client", "cliff", "climb", "clinic", "clip", "clock", "clog", "close", "cloth", "cloud", "clown", "club", "clump", "cluster", "clutch", "coach", "coast", "coconut", "code", "coffee", "coil", "coin", "collect", "color", "column", "combine", "come", "comfort", "comic", "common", "company", "concert", "conduct", "confirm", "congress", "connect", "consider", "control", "convince", "cook", "cool", "copper", "copy", "coral", "core", "corn", "correct", "cost", "cotton", "couch", "country", "couple", "course", "cousin", "cover", "coyote", "crack", "cradle", "craft", "cram", "crane", "crash", "crater", "crawl", "crazy", "cream", "credit", "creek", "crew", "cricket", "crime", "crisp", "critic", "crop", "cross", "crouch", "crowd", "crucial", "cruel", "cruise", "crumble", "crunch", "crush", "cry", "crystal", "cube", "culture", "cup", "cupboard", "curious", "current", "curtain", "curve", "cushion", "custom", "cute", "cycle", "dad", "damage", "damp", "dance", "danger", "daring", "dash", "daughter", "dawn", "day", "deal", "debate", "debris", "decade", "december", "decide", "decline", "decorate", "decrease", "deer", "defense", "define", "defy", "degree", "delay", "deliver", "demand", "demise", "denial", "dentist", "deny", "depart", "depend", "deposit", "depth", "deputy", "derive", "describe", "desert", "design", "desk", "despair", "destroy", "detail", "detect", "develop", "device", "devote", "diagram", "dial", "diamond", "diary", "dice", "diesel", "diet", "differ", "digital", "dignity", "dilemma", "dinner", "dinosaur", "direct", "dirt", "disagree", "discover", "disease", "dish", "dismiss", "disorder", "display", "distance", "divert", "divide", "divorce", "dizzy", "doctor", "document", "dog", "doll", "dolphin", "domain", "donate", "donkey", "donor", "door", "dose", "double", "dove", "draft", "dragon", "drama", "drastic", "draw", "dream", "dress", "drift", "drill", "drink", "drip", "drive", "drop", "drum", "dry", "duck", "dumb", "dune", "during", "dust", "dutch", "duty", "dwarf", "dynamic", "eager", "eagle", "early", "earn", "earth", "easily", "east", "easy", "echo", "ecology", "economy", "edge", "edit", "educate", "effort", "egg", "eight", "either", "elbow", "elder", "electric", "elegant", "element", "elephant", "elevator", "elite", "else", "embark", "embody", "embrace", "emerge", "emotion", "employ", "empower", "empty", "enable", "enact", "end", "endless", "endorse", "enemy", "energy", "enforce", "engage", "engine", "enhance", "enjoy", "enlist", "enough", "enrich", "enroll", "ensure", "enter", "entire", "entry", "envelope", "episode", "equal", "equip", "era", "erase", "erode", "erosion", "error", "erupt", "escape", "essay", "essence", "estate", "eternal", "ethics", "evidence", "evil", "evoke", "evolve", "exact", "example", "excess", "exchange", "excite", "exclude", "excuse", "execute", "exercise", "exhaust", "exhibit", "exile", "exist", "exit", "exotic", "expand", "expect", "expire", "explain", "expose", "express", "extend", "extra", "eye", "eyebrow", "fabric", "face", "faculty", "fade", "faint", "faith", "fall", "false", "fame", "family", "famous", "fan", "fancy", "fantasy", "farm", "fashion", "fat", "fatal", "father", "fatigue", "fault", "favorite", "feature", "february", "federal", "fee", "feed", "feel", "female", "fence", "festival", "fetch", "fever", "few", "fiber", "fiction", "field", "figure", "file", "film", "filter", "final", "find", "fine", "finger", "finish", "fire", "firm", "first", "fiscal", "fish", "fit", "fitness", "fix", "flag", "flame", "flash", "flat", "flavor", "flee", "flight", "flip", "float", "flock", "floor", "flower", "fluid", "flush", "fly", "foam", "focus", "fog", "foil", "fold", "follow", "food", "foot", "force", "forest", "forget", "fork", "fortune", "forum", "forward", "fossil", "foster", "found", "fox", "fragile", "frame", "frequent", "fresh", "friend", "fringe", "frog", "front", "frost", "frown", "frozen", "fruit", "fuel", "fun", "funny", "furnace", "fury", "future", "gadget", "gain", "galaxy", "gallery", "game", "gap", "garage", "garbage", "garden", "garlic", "garment", "gas", "gasp", "gate", "gather", "gauge", "gaze", "general", "genius", "genre", "gentle", "genuine", "gesture", "ghost", "giant", "gift", "giggle", "ginger", "giraffe", "girl", "give", "glad", "glance", "glare", "glass", "glide", "glimpse", "globe", "gloom", "glory", "glove", "glow", "glue", "goat", "goddess", "gold", "good", "goose", "gorilla", "gospel", "gossip", "govern", "gown", "grab", "grace", "grain", "grant", "grape", "grass", "gravity", "great", "green", "grid", "grief", "grit", "grocery", "group", "grow", "grunt", "guard", "guess", "guide", "guilt", "guitar", "gun", "gym", "habit", "hair", "half", "hammer", "hamster", "hand", "happy", "harbor", "hard", "harsh", "harvest", "hat", "have", "hawk", "hazard", "head", "health", "heart", "heavy", "hedgehog", "height", "hello", "helmet", "help", "hen", "hero", "hidden", "high", "hill", "hint", "hip", "hire", "history", "hobby", "hockey", "hold", "hole", "holiday", "hollow", "home", "honey", "hood", "hope", "horn", "horror", "horse", "hospital", "host", "hotel", "hour", "hover", "hub", "huge", "human", "humble", "humor", "hundred", "hungry", "hunt", "hurdle", "hurry", "hurt", "husband", "hybrid", "ice", "icon", "idea", "identify", "idle", "ignore", "ill", "illegal", "illness", "image", "imitate", "immense", "immune", "impact", "impose", "improve", "impulse", "inch", "include", "income", "increase", "index", "indicate", "indoor", "industry", "infant", "inflict", "inform", "inhale", "inherit", "initial", "inject", "injury", "inmate", "inner", "innocent", "input", "inquiry", "insane", "insect", "inside", "inspire", "install", "intact", "interest", "into", "invest", "invite", "involve", "iron", "island", "isolate", "issue", "item", "ivory", "jacket", "jaguar", "jar", "jazz", "jealous", "jeans", "jelly", "jewel", "job", "join", "joke", "journey", "joy", "judge", "juice", "jump", "jungle", "junior", "junk", "just", "kangaroo", "keen", "keep", "ketchup", "key", "kick", "kid", "kidney", "kind", "kingdom", "kiss", "kit", "kitchen", "kite", "kitten", "kiwi", "knee", "knife", "knock", "know", "lab", "label", "labor", "ladder", "lady", "lake", "lamp", "language", "laptop", "large", "later", "latin", "laugh", "laundry", "lava", "law", "lawn", "lawsuit", "layer", "lazy", "leader", "leaf", "learn", "leave", "lecture", "left", "leg", "legal", "legend", "leisure", "lemon", "lend", "length", "lens", "leopard", "lesson", "letter", "level", "liar", "liberty", "library", "license", "life", "lift", "light", "like", "limb", "limit", "link", "lion", "liquid", "list", "little", "live", "lizard", "load", "loan", "lobster", "local", "lock", "logic", "lonely", "long", "loop", "lottery", "loud", "lounge", "love", "loyal", "lucky", "luggage", "lumber", "lunar", "lunch", "luxury", "lyrics", "machine", "mad", "magic", "magnet", "maid", "mail", "main", "major", "make", "mammal", "man", "manage", "mandate", "mango", "mansion", "manual", "maple", "marble", "march", "margin", "marine", "market", "marriage", "mask", "mass", "master", "match", "material", "math", "matrix", "matter", "maximum", "maze", "meadow", "mean", "measure", "meat", "mechanic", "medal", "media", "melody", "melt", "member", "memory", "mention", "menu", "mercy", "merge", "merit", "merry", "mesh", "message", "metal", "method", "middle", "midnight", "milk", "million", "mimic", "mind", "minimum", "minor", "minute", "miracle", "mirror", "misery", "miss", "mistake", "mix", "mixed", "mixture", "mobile", "model", "modify", "mom", "moment", "monitor", "monkey", "monster", "month", "moon", "moral", "more", "morning", "mosquito", "mother", "motion", "motor", "mountain", "mouse", "move", "movie", "much", "muffin", "mule", "multiply", "muscle", "museum", "mushroom", "music", "must", "mutual", "myself", "mystery", "myth", "naive", "name", "napkin", "narrow", "nasty", "nation", "nature", "near", "neck", "need", "negative", "neglect", "neither", "nephew", "nerve", "nest", "net", "network", "neutral", "never", "news", "next", "nice", "night", "noble", "noise", "nominee", "noodle", "normal", "north", "nose", "notable", "note", "nothing", "notice", "novel", "now", "nuclear", "number", "nurse", "nut", "oak", "obey", "object", "oblige", "obscure", "observe", "obtain", "obvious", "occur", "ocean", "october", "odor", "off", "offer", "office", "often", "oil", "okay", "old", "olive", "olympic", "omit", "once", "one", "onion", "online", "only", "open", "opera", "opinion", "oppose", "option", "orange", "orbit", "orchard", "order", "ordinary", "organ", "orient", "original", "orphan", "ostrich", "other", "outdoor", "outer", "output", "outside", "oval", "oven", "over", "own", "owner", "oxygen", "oyster", "ozone", "pact", "paddle", "page", "pair", "palace", "palm", "panda", "panel", "panic", "panther", "paper", "parade", "parent", "park", "parrot", "party", "pass", "patch", "path", "patient", "patrol", "pattern", "pause", "pave", "payment", "peace", "peanut", "pear", "peasant", "pelican", "pen", "penalty", "pencil", "people", "pepper", "perfect", "permit", "person", "pet", "phone", "photo", "phrase", "physical", "piano", "picnic", "picture", "piece", "pig", "pigeon", "pill", "pilot", "pink", "pioneer", "pipe", "pistol", "pitch", "pizza", "place", "planet", "plastic", "plate", "play", "please", "pledge", "pluck", "plug", "plunge", "poem", "poet", "point", "polar", "pole", "police", "pond", "pony", "pool", "popular", "portion", "position", "possible", "post", "potato", "pottery", "poverty", "powder", "power", "practice", "praise", "predict", "prefer", "prepare", "present", "pretty", "prevent", "price", "pride", "primary", "print", "priority", "prison", "private", "prize", "problem", "process", "produce", "profit", "program", "project", "promote", "proof", "property", "prosper", "protect", "proud", "provide", "public", "pudding", "pull", "pulp", "pulse", "pumpkin", "punch", "pupil", "puppy", "purchase", "purity", "purpose", "purse", "push", "put", "puzzle", "pyramid", "quality", "quantum", "quarter", "question", "quick", "quit", "quiz", "quote", "rabbit", "raccoon", "race", "rack", "radar", "radio", "rail", "rain", "raise", "rally", "ramp", "ranch", "random", "range", "rapid", "rare", "rate", "rather", "raven", "raw", "razor", "ready", "real", "reason", "rebel", "rebuild", "recall", "receive", "recipe", "record", "recycle", "reduce", "reflect", "reform", "refuse", "region", "regret", "regular", "reject", "relax", "release", "relief", "rely", "remain", "remember", "remind", "remove", "render", "renew", "rent", "reopen", "repair", "repeat", "replace", "report", "require", "rescue", "resemble", "resist", "resource", "response", "result", "retire", "retreat", "return", "reunion", "reveal", "review", "reward", "rhythm", "rib", "ribbon", "rice", "rich", "ride", "ridge", "rifle", "right", "rigid", "ring", "riot", "ripple", "risk", "ritual", "rival", "river", "road", "roast", "robot", "robust", "rocket", "romance", "roof", "rookie", "room", "rose", "rotate", "rough", "round", "route", "royal", "rubber", "rude", "rug", "rule", "run", "runway", "rural", "sad", "saddle", "sadness", "safe", "sail", "salad", "salmon", "salon", "salt", "salute", "same", "sample", "sand", "satisfy", "satoshi", "sauce", "sausage", "save", "say", "scale", "scan", "scare", "scatter", "scene", "scheme", "school", "science", "scissors", "scorpion", "scout", "scrap", "screen", "script", "scrub", "sea", "search", "season", "seat", "second", "secret", "section", "security", "seed", "seek", "segment", "select", "sell", "seminar", "senior", "sense", "sentence", "series", "service", "session", "settle", "setup", "seven", "shadow", "shaft", "shallow", "share", "shed", "shell", "sheriff", "shield", "shift", "shine", "ship", "shiver", "shock", "shoe", "shoot", "shop", "short", "shoulder", "shove", "shrimp", "shrug", "shuffle", "shy", "sibling", "sick", "side", "siege", "sight", "sign", "silent", "silk", "silly", "silver", "similar", "simple", "since", "sing", "siren", "sister", "situate", "six", "size", "skate", "sketch", "ski", "skill", "skin", "skirt", "skull", "slab", "slam", "sleep", "slender", "slice", "slide", "slight", "slim", "slogan", "slot", "slow", "slush", "small", "smart", "smile", "smoke", "smooth", "snack", "snake", "snap", "sniff", "snow", "soap", "soccer", "social", "sock", "soda", "soft", "solar", "soldier", "solid", "solution", "solve", "someone", "song", "soon", "sorry", "sort", "soul", "sound", "soup", "source", "south", "space", "spare", "spatial", "spawn", "speak", "special", "speed", "spell", "spend", "sphere", "spice", "spider", "spike", "spin", "spirit", "split", "spoil", "sponsor", "spoon", "sport", "spot", "spray", "spread", "spring", "spy", "square", "squeeze", "squirrel", "stable", "stadium", "staff", "stage", "stairs", "stamp", "stand", "start", "state", "stay", "steak", "steel", "stem", "step", "stereo", "stick", "still", "sting", "stock", "stomach", "stone", "stool", "story", "stove", "strategy", "street", "strike", "strong", "struggle", "student", "stuff", "stumble", "style", "subject", "submit", "subway", "success", "such", "sudden", "suffer", "sugar", "suggest", "suit", "summer", "sun", "sunny", "sunset", "super", "supply", "supreme", "sure", "surface", "surge", "surprise", "surround", "survey", "suspect", "sustain", "swallow", "swamp", "swap", "swarm", "swear", "sweet", "swift", "swim", "swing", "switch", "sword", "symbol", "symptom", "syrup", "system", "table", "tackle", "tag", "tail", "talent", "talk", "tank", "tape", "target", "task", "taste", "tattoo", "taxi", "teach", "team", "tell", "ten", "tenant", "tennis", "tent", "term", "test", "text", "thank", "that", "theme", "then", "theory", "there", "they", "thing", "this", "thought", "three", "thrive", "throw", "thumb", "thunder", "ticket", "tide", "tiger", "tilt", "timber", "time", "tiny", "tip", "tired", "tissue", "title", "toast", "tobacco", "today", "toddler", "toe", "together", "toilet", "token", "tomato", "tomorrow", "tone", "tongue", "tonight", "tool", "tooth", "top", "topic", "topple", "torch", "tornado", "tortoise", "toss", "total", "tourist", "toward", "tower", "town", "toy", "track", "trade", "traffic", "tragic", "train", "transfer", "trap", "trash", "travel", "tray", "treat", "tree", "trend", "trial", "tribe", "trick", "trigger", "trim", "trip", "trophy", "trouble", "truck", "true", "truly", "trumpet", "trust", "truth", "try", "tube", "tuition", "tumble", "tuna", "tunnel", "turkey", "turn", "turtle", "twelve", "twenty", "twice", "twin", "twist", "two", "type", "typical", "ugly", "umbrella", "unable", "unaware", "uncle", "uncover", "under", "undo", "unfair", "unfold", "unhappy", "uniform", "unique", "unit", "universe", "unknown", "unlock", "until", "unusual", "unveil", "update", "upgrade", "uphold", "upon", "upper", "upset", "urban", "urge", "usage", "use", "used", "useful", "useless", "usual", "utility", "vacant", "vacuum", "vague", "valid", "valley", "valve", "van", "vanish", "vapor", "various", "vast", "vault", "vehicle", "velvet", "vendor", "venture", "venue", "verb", "verify", "version", "very", "vessel", "veteran", "viable", "vibrant", "vicious", "victory", "video", "view", "village", "vintage", "violin", "virtual", "virus", "visa", "visit", "visual", "vital", "vivid", "vocal", "voice", "void", "volcano", "volume", "vote", "voyage", "wage", "wagon", "wait", "walk", "wall", "walnut", "want", "warfare", "warm", "warrior", "wash", "wasp", "waste", "water", "wave", "way", "wealth", "weapon", "wear", "weasel", "weather", "web", "wedding", "weekend", "weird", "welcome", "west", "wet", "whale", "what", "wheat", "wheel", "when", "where", "whip", "whisper", "wide", "width", "wife", "wild", "will", "win", "window", "wine", "wing", "wink", "winner", "winter", "wire", "wisdom", "wise", "wish", "witness", "wolf", "woman", "wonder", "wood", "wool", "word", "work", "world", "worry", "worth", "wrap", "wreck", "wrestle", "wrist", "write", "wrong", "yard", "year", "yellow", "you", "young", "youth", "zebra", "zero", "zone", "zoo"];
MnemonicUtils.DEFAULT_WORDLIST = MnemonicUtils.ENGLISH_WORDLIST;
((MnemonicUtils2) => {
  let MnemonicType;
  ((MnemonicType2) => {
    MnemonicType2[MnemonicType2["UNKNOWN"] = -1] = "UNKNOWN";
    MnemonicType2[MnemonicType2["LEGACY"] = 0] = "LEGACY";
    MnemonicType2[MnemonicType2["BIP39"] = 1] = "BIP39";
  })(MnemonicType = MnemonicUtils2.MnemonicType || (MnemonicUtils2.MnemonicType = {}));
})(MnemonicUtils || (MnemonicUtils = {}));
Object.freeze(MnemonicUtils);

// lib/Secret.ts
import { CryptoUtils as CryptoUtils3, Hash as Hash2, PrivateKey as PrivateKey2 } from "../../bundler/index.js";
var Secret = class _Secret extends Serializable {
  _type;
  _purposeId;
  static SIZE = 32;
  static ENCRYPTION_SALT_SIZE = 16;
  static ENCRYPTION_KDF_ROUNDS = 256;
  static ENCRYPTION_CHECKSUM_SIZE = 4;
  static ENCRYPTION_CHECKSUM_SIZE_V3 = 2;
  constructor(type, purposeId) {
    super();
    this._type = type;
    this._purposeId = purposeId;
  }
  /**
   * Decrypts a Secret from an encrypted byte array and its password.
   */
  static async fromEncrypted(buf, key) {
    const version = buf.readUint8();
    const roundsLog = buf.readUint8();
    if (roundsLog > 32) throw new Error("Rounds out-of-bounds");
    const rounds = Math.pow(2, roundsLog);
    switch (version) {
      case 3:
        return _Secret._decryptV3(buf, key, rounds);
      default:
        throw new Error("Unsupported version");
    }
  }
  /**
   * Encrypts the Secret with a password.
   */
  async exportEncrypted(key) {
    const salt = CryptoUtils3.getRandomValues(_Secret.ENCRYPTION_SALT_SIZE);
    const data = new SerialBuffer(
      /*purposeId*/
      4 + _Secret.SIZE
    );
    data.writeUint32(this._purposeId);
    data.write(this.serialize());
    const checksum = Hash2.computeBlake2b(data).subarray(0, _Secret.ENCRYPTION_CHECKSUM_SIZE_V3);
    const plaintext = new SerialBuffer(checksum.byteLength + data.byteLength);
    plaintext.write(checksum);
    plaintext.write(data);
    const ciphertext = await CryptoUtils3.otpKdf(plaintext, key, salt, _Secret.ENCRYPTION_KDF_ROUNDS);
    const buf = new SerialBuffer(
      /*version*/
      1 + /*kdf rounds*/
      1 + salt.byteLength + ciphertext.byteLength
    );
    buf.writeUint8(3);
    buf.writeUint8(Math.log2(_Secret.ENCRYPTION_KDF_ROUNDS));
    buf.write(salt);
    buf.write(ciphertext);
    return buf;
  }
  /**
   * Returns the serialized size of this object when encrypted.
   */
  get encryptedSize() {
    return (
      /*version*/
      1 + /*kdf rounds*/
      1 + _Secret.ENCRYPTION_SALT_SIZE + _Secret.ENCRYPTION_CHECKSUM_SIZE_V3 + /*purposeId*/
      4 + _Secret.SIZE
    );
  }
  /**
   * Returns the type of this Secret.
   *
   * - `1: Type.PRIVATE_KEY`
   * - `2: Type.ENTROPY`
   */
  get type() {
    return this._type;
  }
  // private static async _decryptV1(buf: SerialBuffer, key: Uint8Array, rounds: number): Promise<PrivateKey> {
  //     const ciphertext = buf.read(Secret.SIZE);
  //     const salt = buf.read(Secret.ENCRYPTION_SALT_SIZE);
  //     const check = buf.read(Secret.ENCRYPTION_CHECKSUM_SIZE);
  //     const plaintext = await CryptoUtils.otpKdfLegacy(ciphertext, key, salt, rounds);
  //     const privateKey = new PrivateKey(plaintext);
  //     const publicKey = PublicKey.derive(privateKey);
  //     const checksum = publicKey.hash().subarray(0, Secret.ENCRYPTION_CHECKSUM_SIZE);
  //     if (!BufferUtils.equals(check, checksum)) {
  //         throw new Error('Invalid key');
  //     }
  //     return privateKey;
  // }
  // private static async _decryptV2(buf: SerialBuffer, key: Uint8Array, rounds: number): Promise<PrivateKey> {
  //     const ciphertext = buf.read(Secret.SIZE);
  //     const salt = buf.read(Secret.ENCRYPTION_SALT_SIZE);
  //     const check = buf.read(Secret.ENCRYPTION_CHECKSUM_SIZE);
  //     const plaintext = await CryptoUtils.otpKdfLegacy(ciphertext, key, salt, rounds);
  //     const checksum = Hash.computeBlake2b(plaintext).subarray(0, Secret.ENCRYPTION_CHECKSUM_SIZE);
  //     if (!BufferUtils.equals(check, checksum)) {
  //         throw new Error('Invalid key');
  //     }
  //     return new PrivateKey(plaintext);
  // }
  static async _decryptV3(buf, key, rounds) {
    const salt = buf.read(_Secret.ENCRYPTION_SALT_SIZE);
    const ciphertext = buf.read(_Secret.ENCRYPTION_CHECKSUM_SIZE_V3 + /*purposeId*/
    4 + _Secret.SIZE);
    const plaintext = await CryptoUtils3.otpKdf(ciphertext, key, salt, rounds);
    const check = plaintext.subarray(0, _Secret.ENCRYPTION_CHECKSUM_SIZE_V3);
    const payload = plaintext.subarray(_Secret.ENCRYPTION_CHECKSUM_SIZE_V3);
    const checksum = Hash2.computeBlake2b(payload).subarray(0, _Secret.ENCRYPTION_CHECKSUM_SIZE_V3);
    if (!BufferUtils.equals(check, checksum)) {
      throw new Error("Invalid key");
    }
    const purposeId = payload[0] << 24 | payload[1] << 16 | payload[2] << 8 | payload[3];
    const secret = payload.subarray(4);
    switch (purposeId) {
      case PrivateKey2.PURPOSE_ID:
        return new PrivateKey2(secret);
      case Entropy.PURPOSE_ID:
      default:
        return new Entropy(secret);
    }
  }
};
((Secret2) => {
  let Type;
  ((Type2) => {
    Type2[Type2["PRIVATE_KEY"] = 1] = "PRIVATE_KEY";
    Type2[Type2["ENTROPY"] = 2] = "ENTROPY";
  })(Type = Secret2.Type || (Secret2.Type = {}));
})(Secret || (Secret = {}));

// lib/Entropy.ts
var Entropy = class _Entropy extends Secret {
  static SIZE = Secret.SIZE;
  static PURPOSE_ID = 1107296258;
  _obj;
  /**
   * Creates a new Entropy from a byte array.
   */
  constructor(arg) {
    super(Secret.Type.ENTROPY, _Entropy.PURPOSE_ID);
    if (!(arg instanceof Uint8Array)) throw new Error("Primitive: Invalid type");
    if (arg.length !== _Entropy.SIZE) throw new Error("Primitive: Invalid length");
    this._obj = arg;
  }
  /**
   * Generates a new Entropy object from secure randomness.
   */
  static generate() {
    const entropy = CryptoUtils4.getRandomValues(_Entropy.SIZE);
    return new _Entropy(entropy);
  }
  /**
   * Derives an ExtendedPrivateKey from the Entropy.
   */
  toExtendedPrivateKey(password, wordlist) {
    return MnemonicUtils.mnemonicToExtendedPrivateKey(this.toMnemonic(wordlist), password);
  }
  /**
   * Converts the Entropy into a mnemonic.
   */
  toMnemonic(wordlist) {
    return MnemonicUtils.entropyToMnemonic(this, wordlist);
  }
  /**
   * Deserializes an Entropy object from a byte array.
   */
  static unserialize(buf) {
    return new _Entropy(buf.read(_Entropy.SIZE));
  }
  /**
   * Deserializes an Entropy object from a hex string.
   */
  static fromHex(hex) {
    return _Entropy.unserialize(BufferUtils.fromHex(hex));
  }
  /**
   * Serializes the Entropy to a byte array.
   */
  serialize(buf) {
    buf = buf || new SerialBuffer(this.serializedSize);
    buf.write(this._obj);
    return buf;
  }
  /**
   * Returns the serialized size of this Entropy.
   */
  get serializedSize() {
    return _Entropy.SIZE;
  }
  /**
   * Overwrites this Entropy's bytes with a replacement in-memory
   */
  overwrite(entropy) {
    this._obj.set(entropy._obj);
  }
  /**
   * Checks for equality with another Entropy.
   */
  equals(o) {
    return o instanceof _Entropy && super.equals(o);
  }
};
export {
  ArrayUtils,
  BufferUtils,
  Entropy,
  ExtendedPrivateKey,
  MnemonicUtils,
  NumberUtils,
  Secret,
  SerialBuffer,
  StringUtils
};
