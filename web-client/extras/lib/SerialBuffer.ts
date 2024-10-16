import { ArrayUtils } from './ArrayUtils';
import { BufferUtils } from './BufferUtils';
import { NumberUtils } from './NumberUtils';
import { StringUtils } from './StringUtils';

export class SerialBuffer extends Uint8Array {
    private _view: DataView;
    private _readPos: number;
    private _writePos: number;

    static EMPTY: SerialBuffer = new SerialBuffer(0);

    constructor(length: number);
    constructor(array: ArrayLike<number> | ArrayBufferLike);
    constructor(bufferOrArrayOrLength: number | ArrayLike<number> | ArrayBufferLike) {
        super(bufferOrArrayOrLength as any);
        this._view = new DataView(this.buffer);
        this._readPos = 0;
        this._writePos = 0;
    }

    override subarray(start?: number, end?: number): Uint8Array {
        return ArrayUtils.subarray(this, start, end);
    }

    get readPos(): number {
        return this._readPos;
    }

    set readPos(value: number) {
        if (value < 0 || value > this.byteLength) throw `Invalid readPos ${value}`;
        this._readPos = value;
    }

    get writePos(): number {
        return this._writePos;
    }

    set writePos(value: number) {
        if (value < 0 || value > this.byteLength) throw `Invalid writePos ${value}`;
        this._writePos = value;
    }

    /**
     * Resets the read and write position of the buffer to zero.
     */
    reset(): void {
        this._readPos = 0;
        this._writePos = 0;
    }

    read(length: number): Uint8Array {
        const value = this.subarray(this._readPos, this._readPos + length);
        this._readPos += length;
        return new Uint8Array(value);
    }

    write(array: Uint8Array): void {
        this.set(array, this._writePos);
        this._writePos += array.byteLength;
    }

    readUint8(): number {
        return this._view.getUint8(this._readPos++);
    }

    writeUint8(value: number): void {
        this._view.setUint8(this._writePos++, value);
    }

    readUint16(): number {
        const value = this._view.getUint16(this._readPos);
        this._readPos += 2;
        return value;
    }

    writeUint16(value: number): void {
        this._view.setUint16(this._writePos, value);
        this._writePos += 2;
    }

    readUint32(): number {
        const value = this._view.getUint32(this._readPos);
        this._readPos += 4;
        return value;
    }

    writeUint32(value: number): void {
        this._view.setUint32(this._writePos, value);
        this._writePos += 4;
    }

    readUint64(): number {
        const value = this._view.getUint32(this._readPos) * Math.pow(2, 32) + this._view.getUint32(this._readPos + 4);
        if (!NumberUtils.isUint64(value)) throw new Error('Malformed value');
        this._readPos += 8;
        return value;
    }

    writeUint64(value: number): void {
        if (!NumberUtils.isUint64(value)) throw new Error('Malformed value');
        this._view.setUint32(this._writePos, Math.floor(value / Math.pow(2, 32)));
        this._view.setUint32(this._writePos + 4, value);
        this._writePos += 8;
    }

    readVarUint(): number {
        const value = this.readUint8();
        if (value < 0xFD) {
            return value;
        } else if (value === 0xFD) {
            return this.readUint16();
        } else if (value === 0xFE) {
            return this.readUint32();
        } /*if (value === 0xFF)*/ else {
            return this.readUint64();
        }
    }

    writeVarUint(value: number): void {
        if (!NumberUtils.isUint64(value)) throw new Error('Malformed value');
        if (value < 0xFD) {
            this.writeUint8(value);
        } else if (value <= 0xFFFF) {
            this.writeUint8(0xFD);
            this.writeUint16(value);
        } else if (value <= 0xFFFFFFFF) {
            this.writeUint8(0xFE);
            this.writeUint32(value);
        } else {
            this.writeUint8(0xFF);
            this.writeUint64(value);
        }
    }

    static varUintSize(value: number): number {
        if (!NumberUtils.isUint64(value)) throw new Error('Malformed value');
        if (value < 0xFD) {
            return 1;
        } else if (value <= 0xFFFF) {
            return 3;
        } else if (value <= 0xFFFFFFFF) {
            return 5;
        } else {
            return 9;
        }
    }

    readFloat64(): number {
        const value = this._view.getFloat64(this._readPos);
        this._readPos += 8;
        return value;
    }

    writeFloat64(value: number): void {
        this._view.setFloat64(this._writePos, value);
        this._writePos += 8;
    }

    readString(length: number): string {
        const bytes = this.read(length);
        return BufferUtils.toUtf8(bytes);
    }

    writeString(value: string, length: number): void {
        if (!StringUtils.isWellFormed(value) || value.length !== length) throw new Error('Malformed value/length');
        const bytes = BufferUtils.fromUtf8(value);
        this.write(bytes);
    }

    readPaddedString(length: number): string {
        const bytes = this.read(length);
        let i = 0;
        while (i < length && bytes[i] !== 0x0) i++;
        const view = new Uint8Array(bytes.buffer, bytes.byteOffset, i);
        return BufferUtils.toUtf8(view);
    }

    writePaddedString(value: string, length: number): void {
        if (!StringUtils.isWellFormed(value) || value.length > length) throw new Error('Malformed value/length');
        const bytes = BufferUtils.fromUtf8(value);
        this.write(bytes);
        const padding = length - bytes.byteLength;
        this.write(new Uint8Array(padding));
    }

    readVarLengthString(): string {
        const length = this.readUint8();
        if (this._readPos + length > this.length) throw new Error('Malformed length');
        const bytes = this.read(length);
        return BufferUtils.toUtf8(bytes);
    }

    writeVarLengthString(value: string): void {
        if (!StringUtils.isWellFormed(value) || !NumberUtils.isUint8(value.length)) throw new Error('Malformed value');
        const bytes = BufferUtils.fromUtf8(value);
        this.writeUint8(bytes.byteLength);
        this.write(bytes);
    }

    static varLengthStringSize(value: string): number {
        if (!StringUtils.isWellFormed(value) || !NumberUtils.isUint8(value.length)) throw new Error('Malformed value');
        return /*length*/ 1 + value.length;
    }
}
