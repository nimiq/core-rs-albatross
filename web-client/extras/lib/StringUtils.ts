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

    static commonPrefix(str1: string, str2: string): string {
        let i = 0;
        for (; i < str1.length; ++i) {
            if (str1[i] !== str2[i]) break;
        }
        return str1.substring(0, i);
    }

    static lpad(str: string, padString: string, length: number): string {
        while (str.length < length) str = padString + str;
        return str;
    }
}
