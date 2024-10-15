import { CryptoUtils } from '@nimiq/core';
import { BufferUtils } from './BufferUtils';
import { ExtendedPrivateKey } from './ExtendedPrivateKey';
import { MnemonicUtils } from './MnemonicUtils';
import { Secret } from './Secret';
import { SerialBuffer } from './SerialBuffer';

export class Entropy extends Secret {
    static override SIZE = Secret.SIZE;
    static PURPOSE_ID = 0x42000002;

    private _obj: Uint8Array;

    /**
     * Creates a new Entropy from a byte array.
     */
    constructor(arg: Uint8Array) {
        super(Secret.Type.ENTROPY, Entropy.PURPOSE_ID);
        if (!(arg instanceof Uint8Array)) throw new Error('Primitive: Invalid type');
        if (arg.length !== Entropy.SIZE) throw new Error('Primitive: Invalid length');
        this._obj = arg;
    }

    /**
     * Generates a new Entropy object from secure randomness.
     */
    static generate(): Entropy {
        const entropy = CryptoUtils.getRandomValues(Entropy.SIZE);
        return new Entropy(entropy);
    }

    /**
     * Derives an ExtendedPrivateKey from the Entropy.
     */
    toExtendedPrivateKey(password?: string, wordlist?: string[]): ExtendedPrivateKey {
        return MnemonicUtils.mnemonicToExtendedPrivateKey(this.toMnemonic(wordlist), password);
    }

    /**
     * Converts the Entropy into a mnemonic.
     */
    toMnemonic(wordlist?: string[]): string[] {
        return MnemonicUtils.entropyToMnemonic(this, wordlist);
    }

    /**
     * Deserializes an Entropy object from a byte array.
     */
    static deserialize(buf: SerialBuffer): Entropy {
        return new Entropy(buf.read(Entropy.SIZE));
    }

    /**
     * Deserializes an Entropy object from a hex string.
     */
    static fromHex(hex: string): Entropy {
        return Entropy.deserialize(BufferUtils.fromHex(hex));
    }

    /**
     * Serializes the Entropy to a byte array.
     */
    serialize(buf?: SerialBuffer): SerialBuffer {
        buf = buf || new SerialBuffer(this.serializedSize);
        buf.write(this._obj);
        return buf;
    }

    /**
     * Returns the serialized size of this Entropy.
     */
    get serializedSize(): number {
        return Entropy.SIZE;
    }

    /**
     * Overwrites this Entropy's bytes with a replacement in-memory
     */
    overwrite(entropy: Entropy): void {
        this._obj.set(entropy._obj);
    }

    /**
     * Checks for equality with another Entropy.
     */
    override equals(o: unknown): boolean {
        return o instanceof Entropy && super.equals(o);
    }
}
