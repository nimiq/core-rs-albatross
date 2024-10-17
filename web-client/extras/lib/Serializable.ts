import { BufferUtils } from './BufferUtils';
import { SerialBuffer } from './SerialBuffer';

export abstract class Serializable {
    /**
     * Checks for equality with another Serializable.
     */
    equals(o: unknown): boolean {
        return o instanceof Serializable && BufferUtils.equals(this.serialize(), o.serialize());
    }

    /**
     * Compares this object to another object.
     *
     * Returns a negative number if `this` is smaller than o, a positive number if `this` is larger than o, and zero if equal.
     */
    compare(o: Serializable): number {
        return BufferUtils.compare(this.serialize(), o.serialize());
    }

    abstract serialize(buf?: SerialBuffer): SerialBuffer;

    /**
     * Formats the object into a hex string.
     */
    toString(): string {
        return this.toHex();
    }

    /**
     * Formats the object into a base64 string.
     */
    toBase64(): string {
        return BufferUtils.toBase64(this.serialize());
    }

    /**
     * Formats the object into a hex string.
     */
    toHex(): string {
        return BufferUtils.toHex(this.serialize());
    }
}
