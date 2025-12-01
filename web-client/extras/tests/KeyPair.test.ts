import { KeyPair, PrivateKey } from '@nimiq/core';
import { describe, expect, it } from 'vitest';

describe('KeyPair', () => {
    it('can roundtrip toHex/fromHex', () => {
        const kp1 = KeyPair.generate();
        const hex = kp1.toHex();
        const kp2 = KeyPair.fromHex(hex);

        expect(kp2.privateKey.toHex()).toBe(kp1.privateKey.toHex());
        expect(kp2.publicKey.toHex()).toBe(kp1.publicKey.toHex());
    });

    it('accepts 128-char hex (no padding)', () => {
        const kp1 = KeyPair.generate();
        const hex = kp1.privateKey.toHex() + kp1.publicKey.toHex();

        expect(hex.length).toBe(128);
        const kp2 = KeyPair.fromHex(hex);
        expect(kp2.privateKey.toHex()).toBe(kp1.privateKey.toHex());
    });

    it('rejects short hex strings', () => {
        expect(() => KeyPair.fromHex('a'.repeat(100))).toThrow();
    });

    it('derives from PrivateKey', () => {
        const priv = PrivateKey.generate();
        const kp = KeyPair.derive(priv);
        expect(kp.privateKey.toHex()).toBe(priv.toHex());
    });
});
