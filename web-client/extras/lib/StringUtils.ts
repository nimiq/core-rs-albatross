export class StringUtils {
    static isMultibyte(str: string): boolean {
        return /[\uD800-\uDFFF]/.test(str);
    }

    static isHex(str: string): boolean {
        return /^[0-9A-Fa-f]*$/.test(str);
    }

    static isHexBytes(str: string, length?: number): boolean {
        if (!StringUtils.isHex(str)) return false;
        if (str.length % 2 !== 0) return false;
        if (typeof length === 'number' && str.length / 2 !== length) return false;
        return true;
    }
}
