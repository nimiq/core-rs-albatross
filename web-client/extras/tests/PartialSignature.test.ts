import { CommitmentPair, PartialSignature, PrivateKey, PublicKey } from '@nimiq/core';
import { describe, expect, it } from 'vitest';

describe('PartialSignature', () => {
    it('can create a partial signature', () => {
        const privateKey = PrivateKey.generate();
        const publicKey = PublicKey.derive(privateKey);

        const commitmentPairs = [
            CommitmentPair.generate(),
            CommitmentPair.generate(),
        ];

        const publicKeys = [
            PublicKey.derive(PrivateKey.generate()),
            PublicKey.derive(PrivateKey.generate()),
        ];

        const commitments = [
            [
                CommitmentPair.generate().commitment,
                CommitmentPair.generate().commitment,
            ],
            [
                CommitmentPair.generate().commitment,
                CommitmentPair.generate().commitment,
            ],
        ];

        const partial_signature = PartialSignature.create(
            privateKey,
            publicKey,
            commitmentPairs,
            publicKeys,
            commitments,
            new Uint8Array(139),
        );

        expect(partial_signature.serialize().length).toBe(32);
    });

    it('can create a partial signature without other signers', () => {
        const privateKey = PrivateKey.generate();
        const publicKey = PublicKey.derive(privateKey);

        const commitmentPairs = [
            CommitmentPair.generate(),
            CommitmentPair.generate(),
        ];

        const partial_signature = PartialSignature.create(
            privateKey,
            publicKey,
            commitmentPairs,
            [],
            [],
            new Uint8Array(139),
        );

        expect(partial_signature.serialize().length).toBe(32);
    });
});
