export class StringUtils {
    static isWellFormed(str: string): boolean {
        return typeof str.isWellFormed === 'function' ? str.isWellFormed() : StringUtils._isWellFormedUnicode(str);
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

    /**
     * Copied from:
     * https://github.com/ljharb/es-abstract/blob/main/2024/IsStringWellFormedUnicode.js
     */
    private static _isWellFormedUnicode(string: string): boolean {
        var len = string.length; // step 1
        var k = 0; // step 2
        while (k < len) { // step 3
            var cp = StringUtils._codePointAt(string, k); // step 3.a
            if (cp['[[IsUnpairedSurrogate]]']) {
                return false; // step 3.b
            }
            k += cp['[[CodeUnitCount]]']; // step 3.c
        }
        return true; // step 4
    }

    /**
     * Copied from:
     * https://github.com/ljharb/es-abstract/blob/main/2024/CodePointAt.js
     */
    private static _codePointAt(string: string, position: number) {
        var size = string.length;
        if (position < 0 || position >= size) {
            throw new Error('Assertion failed: `position` must be >= 0, and < the length of `string`');
        }
        var first = string.charCodeAt(position);
        var cp = string.charAt(position);
        var firstIsLeading = StringUtils._isLeadingSurrogate(first);
        var firstIsTrailing = StringUtils._isTrailingSurrogate(first);
        if (!firstIsLeading && !firstIsTrailing) {
            return {
                '[[CodePoint]]': cp,
                '[[CodeUnitCount]]': 1,
                '[[IsUnpairedSurrogate]]': false,
            };
        }
        if (firstIsTrailing || (position + 1 === size)) {
            return {
                '[[CodePoint]]': cp,
                '[[CodeUnitCount]]': 1,
                '[[IsUnpairedSurrogate]]': true,
            };
        }
        var second = string.charCodeAt(position + 1);
        if (!StringUtils._isTrailingSurrogate(second)) {
            return {
                '[[CodePoint]]': cp,
                '[[CodeUnitCount]]': 1,
                '[[IsUnpairedSurrogate]]': true,
            };
        }

        return {
            '[[CodePoint]]': StringUtils._utf16SurrogatePairToCodePoint(first, second),
            '[[CodeUnitCount]]': 2,
            '[[IsUnpairedSurrogate]]': false,
        };
    }

    /**
     * Copied from:
     * https://github.com/ljharb/es-abstract/blob/main/helpers/isLeadingSurrogate.js
     */
    private static _isLeadingSurrogate(charCode: number): boolean {
        return charCode >= 0xD800 && charCode <= 0xDBFF;
    }

    /**
     * Copied from:
     * https://github.com/ljharb/es-abstract/blob/main/helpers/isTrailingSurrogate.js
     */
    private static _isTrailingSurrogate(charCode: number): boolean {
        return charCode >= 0xDC00 && charCode <= 0xDFFF;
    }

    /**
     * Copied from:
     * https://github.com/ljharb/es-abstract/blob/main/2024/UTF16SurrogatePairToCodePoint.js
     */
    private static _utf16SurrogatePairToCodePoint(lead: number, trail: number): string {
        if (!StringUtils._isLeadingSurrogate(lead) || !StringUtils._isTrailingSurrogate(trail)) {
            throw new Error(
                'Assertion failed: `lead` must be a leading surrogate char code, and `trail` must be a trailing surrogate char code',
            );
        }
        // var cp = (lead - 0xD800) * 0x400 + (trail - 0xDC00) + 0x10000;
        return String.fromCharCode(lead) + String.fromCharCode(trail);
    }
}
