import { Commitment, CommitmentPair, RandomSecret } from '@nimiq/core';
import { describe, expect, it } from 'vitest';

describe('CommitmentPair', () => {
    it('can derive the correct commitment from a random secret', () => {
        const pair = CommitmentPair.generate();

        const randomSecret = pair.secret;
        expect(Commitment.derive(randomSecret).equals(pair.commitment)).toBe(true);
        expect(CommitmentPair.derive(randomSecret).equals(pair)).toBe(true);
    });
});
