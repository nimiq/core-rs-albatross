export class CRC8 {
    private static _table: number[] | null = null;

    // Adapted from https://github.com/mode80/crc8js
    private static _createTable() {
        // Create a lookup table byte array
        const table = []; // 256 max len byte array

        for (let i = 0; i < 256; ++i) {
            let curr = i;
            for (let j = 0; j < 8; ++j) {
                if ((curr & 0x80) !== 0) {
                    curr = ((curr << 1) ^ 0x97) % 256; // Polynomial C2 by Baicheva98
                } else {
                    curr = (curr << 1) % 256;
                }
            }
            table[i] = curr;
        }
        return table;
    }

    static compute(buf: Uint8Array): number {
        if (!CRC8._table) CRC8._table = CRC8._createTable();
        // Calculate the 8-bit checksum given an array of byte-sized numbers
        let c = 0;
        for (let i = 0; i < buf.length; i++) {
            c = CRC8._table[(c ^ buf[i]) % 256];
        }
        return c;
    }
}
