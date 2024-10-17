import { Address, CryptoUtils, PrivateKey, PublicKey } from '@nimiq/core';
import { BufferUtils } from './BufferUtils';
import { NumberUtils } from './NumberUtils';
import { SerialBuffer } from './SerialBuffer';
import { Serializable } from './Serializable';

export class ExtendedPrivateKey extends Serializable {
    static CHAIN_CODE_SIZE = 32;

    private _key: PrivateKey;
    private _chainCode: Uint8Array;

    /**
     * Creates an ExtendedPrivateKey from a private key and chain code.
     */
    constructor(key: PrivateKey, chainCode: Uint8Array) {
        super();
        if (!(key instanceof PrivateKey)) throw new Error('ExtendedPrivateKey: Invalid key');
        if (!(chainCode instanceof Uint8Array)) throw new Error('ExtendedPrivateKey: Invalid chainCode');
        if (chainCode.length !== ExtendedPrivateKey.CHAIN_CODE_SIZE) {
            throw new Error('ExtendedPrivateKey: Invalid chainCode length');
        }
        this._key = key;
        this._chainCode = chainCode;
    }

    /**
     * Generates the master ExtendedPrivateKey from a seed.
     */
    static generateMasterKey(seed: Uint8Array): ExtendedPrivateKey {
        const bCurve = BufferUtils.fromUtf8('ed25519 seed');
        const hash = CryptoUtils.computeHmacSha512(bCurve, seed);
        return new ExtendedPrivateKey(new PrivateKey(hash.slice(0, 32)), hash.slice(32));
    }

    /**
     * Derives a child ExtendedPrivateKey from the current key at the provided index.
     */
    derive(index: number): ExtendedPrivateKey {
        // Only hardened derivation is allowed for ed25519.
        if (index < 0x80000000) index += 0x80000000;

        const data = new SerialBuffer(1 + PrivateKey.SIZE + 4);
        data.writeUint8(0);
        data.write(this._key.serialize());
        data.writeUint32(index);

        const hash = CryptoUtils.computeHmacSha512(this._chainCode, data);
        return new ExtendedPrivateKey(new PrivateKey(hash.slice(0, 32)), hash.slice(32));
    }

    /**
     * Tests if a HD derivation path is valid.
     */
    static isValidPath(path: string): boolean {
        if (path.match(/^m(\/[0-9]+')*$/) === null) return false;

        // Overflow check.
        const segments = path.split('/');
        for (let i = 1; i < segments.length; i++) {
            if (!NumberUtils.isUint32(parseInt(segments[i]))) return false;
        }

        return true;
    }

    /**
     * Derives a child ExtendedPrivateKey from the current key at the provided path.
     */
    derivePath(path: string): ExtendedPrivateKey {
        if (!ExtendedPrivateKey.isValidPath(path)) throw new Error('Invalid path');

        let extendedKey: ExtendedPrivateKey = this;
        const segments = path.split('/');
        for (let i = 1; i < segments.length; i++) {
            const index = parseInt(segments[i]);
            extendedKey = extendedKey.derive(index);
        }
        return extendedKey;
    }

    /**
     * Derives an ExtendedPrivateKey from a seed and a derivation path.
     */
    static derivePathFromSeed(path: string, seed: Uint8Array): ExtendedPrivateKey {
        let extendedKey = ExtendedPrivateKey.generateMasterKey(seed);
        return extendedKey.derivePath(path);
    }

    /**
     * Deserializes an ExtendedPrivateKey from a byte array.
     */
    static deserialize(buf: SerialBuffer): ExtendedPrivateKey {
        const privateKey = PrivateKey.deserialize(buf);
        const chainCode = buf.read(ExtendedPrivateKey.CHAIN_CODE_SIZE);
        return new ExtendedPrivateKey(privateKey, chainCode);
    }

    /**
     * Deserializes an ExtendedPrivateKey from a hex string.
     */
    static fromHex(hex: string): ExtendedPrivateKey {
        return ExtendedPrivateKey.deserialize(BufferUtils.fromHex(hex));
    }

    /**
     * Serializes the ExtendedPrivateKey to a byte array.
     */
    serialize(buf?: SerialBuffer): SerialBuffer {
        buf = buf || new SerialBuffer(this.serializedSize);
        buf.write(this._key.serialize());
        buf.write(this._chainCode);
        return buf;
    }

    /**
     * Returns the serialized size of this ExtendedPrivateKey.
     */
    get serializedSize(): number {
        return this._key.serializedSize + ExtendedPrivateKey.CHAIN_CODE_SIZE;
    }

    /**
     * Checks for equality with another ExtendedPrivateKey.
     */
    override equals(o: unknown): boolean {
        return o instanceof ExtendedPrivateKey && super.equals(o);
    }

    /**
     * Returns the private key of this ExtendedPrivateKey.
     */
    get privateKey(): PrivateKey {
        return this._key;
    }

    /**
     * Returns the chain code of this ExtendedPrivateKey.
     */
    get chainCode(): Uint8Array {
        return this._chainCode;
    }

    /**
     * Returns the address related to this ExtendedPrivateKey.
     */
    toAddress(): Address {
        return PublicKey.derive(this._key).toAddress();
    }
}
