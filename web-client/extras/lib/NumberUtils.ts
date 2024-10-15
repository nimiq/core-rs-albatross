export class NumberUtils {
    static UINT8_MAX = 255;
    static UINT16_MAX = 65535;
    static UINT32_MAX = 4294967295;
    static UINT64_MAX = Number.MAX_SAFE_INTEGER;

    static isInteger(val: unknown): val is number {
        return Number.isInteger(val);
    }

    static isUint8(val: unknown): boolean {
        return NumberUtils.isInteger(val)
            && val >= 0 && val <= NumberUtils.UINT8_MAX;
    }

    static isUint16(val: unknown): boolean {
        return NumberUtils.isInteger(val)
            && val >= 0 && val <= NumberUtils.UINT16_MAX;
    }

    static isUint32(val: unknown): boolean {
        return NumberUtils.isInteger(val)
            && val >= 0 && val <= NumberUtils.UINT32_MAX;
    }

    static isUint64(val: unknown): boolean {
        return NumberUtils.isInteger(val)
            && val >= 0 && val <= NumberUtils.UINT64_MAX;
    }
}
