import { PrivateKey, Address } from '@nimiq/core';

declare class ArrayUtils {
    static randomElement<T>(arr: T[]): T;
    static subarray(uintarr: Uint8Array, begin?: number, end?: number): Uint8Array;
    static k_combinations(list: any[], k: number): Generator<any[]>;
}

declare class SerialBuffer extends Uint8Array {
    private _view;
    private _readPos;
    private _writePos;
    static EMPTY: SerialBuffer;
    constructor(length: number);
    constructor(array: ArrayLike<number> | ArrayBufferLike);
    subarray(start?: number, end?: number): Uint8Array;
    get readPos(): number;
    set readPos(value: number);
    get writePos(): number;
    set writePos(value: number);
    /**
     * Resets the read and write position of the buffer to zero.
     */
    reset(): void;
    read(length: number): Uint8Array;
    write(array: Uint8Array): void;
    readUint8(): number;
    writeUint8(value: number): void;
    readUint16(): number;
    writeUint16(value: number): void;
    readUint32(): number;
    writeUint32(value: number): void;
    readUint64(): number;
    writeUint64(value: number): void;
    readVarUint(): number;
    writeVarUint(value: number): void;
    static varUintSize(value: number): number;
    readFloat64(): number;
    writeFloat64(value: number): void;
    readString(length: number): string;
    writeString(value: string, length: number): void;
    readPaddedString(length: number): string;
    writePaddedString(value: string, length: number): void;
    readVarLengthString(): string;
    writeVarLengthString(value: string): void;
    static varLengthStringSize(value: string): number;
}

type TypedArray = Int8Array | Uint8Array | Int16Array | Uint16Array | Int32Array | Uint32Array | Uint8ClampedArray | Float32Array | Float64Array;
declare class BufferUtils {
    static BASE64_ALPHABET: string;
    static BASE32_ALPHABET: {
        RFC4648: string;
        RFC4648_HEX: string;
        NIMIQ: string;
    };
    static HEX_ALPHABET: string;
    static _BASE64_LOOKUP: string[];
    private static _ISO_8859_15_DECODER?;
    private static _UTF8_ENCODER?;
    static toAscii(buffer: Uint8Array): string;
    static fromAscii(string: string): Uint8Array;
    static _codePointTextDecoder(buffer: Uint8Array): string;
    static _tripletToBase64(num: number): string;
    static _base64encodeChunk(u8: Uint8Array, start: number, end: number): string;
    static _base64fromByteArray(u8: Uint8Array): string;
    static toBase64(buffer: Uint8Array): string;
    static fromBase64(base64: string, length?: number): SerialBuffer;
    static toBase64Url(buffer: Uint8Array): string;
    static fromBase64Url(base64: string, length?: number): SerialBuffer;
    static toBase32(buf: Uint8Array, alphabet?: string): string;
    static fromBase32(base32: string, alphabet?: string): Uint8Array;
    static toHex(buffer: Uint8Array): string;
    static fromHex(hex: string, length?: number): SerialBuffer;
    static toBinary(buffer: ArrayLike<number>): string;
    private static _strToUint8Array;
    private static _utf8TextEncoder;
    static fromUtf8(str: string): Uint8Array;
    static fromAny(o: Uint8Array | string, length?: number): SerialBuffer;
    static concatTypedArrays<T extends TypedArray>(a: T, b: T): T;
    static equals(a: TypedArray, b: TypedArray): boolean;
    /**
     * Returns -1 if a is smaller than b, 1 if a is larger than b, 0 if a equals b.
     */
    static compare(a: TypedArray, b: TypedArray): number;
    static xor(a: Uint8Array, b: Uint8Array): Uint8Array;
    private static _toUint8View;
}

declare abstract class Serializable {
    /**
     * Checks for equality with another Serializable.
     */
    equals(o: unknown): boolean;
    /**
     * Compares this object to another object.
     *
     * Returns a negative number if `this` is smaller than o, a positive number if `this` is larger than o, and zero if equal.
     */
    compare(o: Serializable): number;
    abstract serialize(buf?: SerialBuffer): SerialBuffer;
    /**
     * Formats the object into a hex string.
     */
    toString(): string;
    /**
     * Formats the object into a base64 string.
     */
    toBase64(): string;
    /**
     * Formats the object into a hex string.
     */
    toHex(): string;
}

declare class ExtendedPrivateKey extends Serializable {
    static CHAIN_CODE_SIZE: number;
    private _key;
    private _chainCode;
    /**
     * Creates an ExtendedPrivateKey from a private key and chain code.
     */
    constructor(key: PrivateKey, chainCode: Uint8Array);
    /**
     * Generates the master ExtendedPrivateKey from a seed.
     */
    static generateMasterKey(seed: Uint8Array): ExtendedPrivateKey;
    /**
     * Derives a child ExtendedPrivateKey from the current key at the provided index.
     */
    derive(index: number): ExtendedPrivateKey;
    /**
     * Tests if a HD derivation path is valid.
     */
    static isValidPath(path: string): boolean;
    /**
     * Derives a child ExtendedPrivateKey from the current key at the provided path.
     */
    derivePath(path: string): ExtendedPrivateKey;
    /**
     * Derives an ExtendedPrivateKey from a seed and a derivation path.
     */
    static derivePathFromSeed(path: string, seed: Uint8Array): ExtendedPrivateKey;
    /**
     * Deserializes an ExtendedPrivateKey from a byte array.
     */
    static unserialize(buf: SerialBuffer): ExtendedPrivateKey;
    /**
     * Deserializes an ExtendedPrivateKey from a hex string.
     */
    static fromHex(hex: string): ExtendedPrivateKey;
    /**
     * Serializes the ExtendedPrivateKey to a byte array.
     */
    serialize(buf?: SerialBuffer): SerialBuffer;
    /**
     * Returns the serialized size of this ExtendedPrivateKey.
     */
    get serializedSize(): number;
    /**
     * Checks for equality with another ExtendedPrivateKey.
     */
    equals(o: unknown): boolean;
    /**
     * Returns the private key of this ExtendedPrivateKey.
     */
    get privateKey(): PrivateKey;
    /**
     * Returns the address related to this ExtendedPrivateKey.
     */
    toAddress(): Address;
}

declare abstract class Secret extends Serializable {
    private _type;
    private _purposeId;
    static SIZE: number;
    static ENCRYPTION_SALT_SIZE: number;
    static ENCRYPTION_KDF_ROUNDS: number;
    static ENCRYPTION_CHECKSUM_SIZE: number;
    static ENCRYPTION_CHECKSUM_SIZE_V3: number;
    constructor(type: Secret.Type, purposeId: number);
    /**
     * Decrypts a Secret from an encrypted byte array and its password.
     */
    static fromEncrypted(buf: SerialBuffer, key: Uint8Array): Promise<PrivateKey | Entropy>;
    /**
     * Encrypts the Secret with a password.
     */
    exportEncrypted(key: Uint8Array): Promise<SerialBuffer>;
    /**
     * Returns the serialized size of this object when encrypted.
     */
    get encryptedSize(): number;
    /**
     * Returns the type of this Secret.
     *
     * - `1: Type.PRIVATE_KEY`
     * - `2: Type.ENTROPY`
     */
    get type(): Secret.Type;
    private static _decryptV3;
}
declare namespace Secret {
    enum Type {
        PRIVATE_KEY = 1,
        ENTROPY = 2
    }
}

declare class Entropy extends Secret {
    static SIZE: number;
    static PURPOSE_ID: number;
    private _obj;
    /**
     * Creates a new Entropy from a byte array.
     */
    constructor(arg: Uint8Array);
    /**
     * Generates a new Entropy object from secure randomness.
     */
    static generate(): Entropy;
    /**
     * Derives an ExtendedPrivateKey from the Entropy.
     */
    toExtendedPrivateKey(password?: string, wordlist?: string[]): ExtendedPrivateKey;
    /**
     * Converts the Entropy into a mnemonic.
     */
    toMnemonic(wordlist?: string[]): string[];
    /**
     * Deserializes an Entropy object from a byte array.
     */
    static unserialize(buf: SerialBuffer): Entropy;
    /**
     * Deserializes an Entropy object from a hex string.
     */
    static fromHex(hex: string): Entropy;
    /**
     * Serializes the Entropy to a byte array.
     */
    serialize(buf?: SerialBuffer): SerialBuffer;
    /**
     * Returns the serialized size of this Entropy.
     */
    get serializedSize(): number;
    /**
     * Overwrites this Entropy's bytes with a replacement in-memory
     */
    overwrite(entropy: Entropy): void;
    /**
     * Checks for equality with another Entropy.
     */
    equals(o: unknown): boolean;
}

declare class MnemonicUtils {
    /**
     * The English wordlist.
     */
    static ENGLISH_WORDLIST: string[];
    /**
     * The default English wordlist.
     */
    static DEFAULT_WORDLIST: string[];
    /**
     * Converts an Entropy to a mnemonic.
     */
    static entropyToMnemonic(entropy: string | ArrayBuffer | Uint8Array | Entropy, wordlist?: string[]): string[];
    /**
     * Converts a mnemonic to an Entropy.
     */
    static mnemonicToEntropy(mnemonic: string[] | string, wordlist?: string[]): Entropy;
    /**
     * Converts a mnemonic to a seed.
     *
     * Optionally takes a password to use for the seed derivation.
     */
    static mnemonicToSeed(mnemonic: string[] | string, password?: string): SerialBuffer;
    /**
     * Converts a mnemonic to an extended private key.
     *
     * Optionally takes a password to use for the seed derivation.
     */
    static mnemonicToExtendedPrivateKey(mnemonic: string[] | string, password?: string): ExtendedPrivateKey;
    /**
     * Tests if a mnemonic can be both for a legacy Nimiq wallet and a BIP39 wallet.
     */
    static isCollidingChecksum(entropy: Entropy): boolean;
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
    static getMnemonicType(mnemonic: string[] | string, wordlist?: string[]): MnemonicUtils.MnemonicType;
    private static _crcChecksum;
    private static _sha256Checksum;
    private static _entropyToBits;
    private static _normalizeEntropy;
    private static _bitsToMnemonic;
    private static _mnemonicToBits;
    private static _bitsToEntropy;
    private static _salt;
}
declare namespace MnemonicUtils {
    enum MnemonicType {
        UNKNOWN = -1,
        LEGACY = 0,
        BIP39 = 1
    }
}

declare class NumberUtils {
    static UINT8_MAX: number;
    static UINT16_MAX: number;
    static UINT32_MAX: number;
    static UINT64_MAX: number;
    static isInteger(val: unknown): val is number;
    static isUint8(val: unknown): boolean;
    static isUint16(val: unknown): boolean;
    static isUint32(val: unknown): boolean;
    static isUint64(val: unknown): boolean;
    static randomUint32(): number;
    static randomUint64(): number;
    static fromBinary(bin: string): number;
}

declare class StringUtils {
    static isMultibyte(str: string): boolean;
    static isHex(str: string): boolean;
    static isHexBytes(str: string, length?: number): boolean;
    static commonPrefix(str1: string, str2: string): string;
    static lpad(str: string, padString: string, length: number): string;
}

export { ArrayUtils, BufferUtils, Entropy, ExtendedPrivateKey, MnemonicUtils, NumberUtils, Secret, SerialBuffer, StringUtils };
