import {
    Address,
    Commitment,
    CommitmentPair,
    KeyPair,
    PartialSignature,
    PublicKey,
    SignatureProof,
    TransactionBuilder,
} from '@nimiq/core';
import { describe, expect, it } from 'vitest';

describe('MuSig2', async () => {
    it('can create a 2-of-3 multi-signature', () => {
        // Test vectors
        const _keypair_a = "14a3bc3b25c73b6ca3e829aef329a2a6dc69ae52b8d20a164831a021b6a9f9feec98d39d98a58c13d399673d6da7dc6c74f379eddd8c8628e40ffc6be7c2498300";
        const _keypair_b = "2da15ede9992fad834b73283dd1a24f5a7a52b067b09be132ddb5232df863125bb639b6bbf6db003a94a83ef9d12f12fcc5990f63954b7f6d88f5be58f8c411200";
        const _keypair_c = "a3b3d799e7fca4baa3568d58e0c909af1f832926020163a1d48998621a15c9c6b81b12bcb1a6e9ba49a6dec268705c2cc2d70d1d7e22493a4128559eadacdbd400";
        const _wallet_address = "NQ77 XKHG BUSE L76F 030F FY5U 0C6H 6HXU BSPX";

        const _commitment_pair_a_1 = "61514436ba3671457a39ab8b89c166a6dbf9dcf2320142412faca62c0e30180ec441e06b23ef64095dd24ba9976e1bd6086dd34f6d2892ec92c8f3a5365e352f";
        const _commitment_pair_a_2 = "246a60bacd6be35bc248de42bd8d8035c66766af037859797a3c6c87475fc20a6af6931e2199aa73707d1e2363502af6a637a33ddc9464b5a60dab9c5535240d";

        const _commitment_pair_b_1 = "1c25176a8d9531dfdabd393e24457ef768b8f91ad1aa5b5c5d531c59d61493068bcf3923fe74da2c0dae83a0f0a4ad78c3ace4737e1bab09ae839059cc06b75a";
        const _commitment_pair_b_2 = "9d372fe33120b7555f06112efa51a179e745ae03cc0942319a0b2a605c680708170b6a773e7f633ef7c3830ebe16a4a7dde24ba4040c18b361b6aa5fad2d0e6f";

        const _signing_public_key = "768aa1e50751d31c7e16708903f6906621da68fd1daae12210480dac10d8a57b";
        const _aggregate_commitment = "f52764eed6c6f89f6a07781f035707aab471e846a9b815c73d0eb620cf345b82";
        const _b_scalar = "9556574afd98401d38bee97e9ecc70a3d0d29f3221fd4b73a71f6444678b3306";

        const _partial_signature_a = "0f751ab3db73576994159919a970b529a68e0c1f5b49501243c573106cff200d";
        const _partial_signature_b = "db2cd118d2e46fed51cff0341a139b90159a134095d7a003671d71d53a33520c";

        const _unsigned_transaction = "01f4e305f34ea1ccf00c0f7fcbc030d1347dc5eafe000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000050000";
        const _signed_transaction = "01f4e305f34ea1ccf00c0f7fcbc030d1347dc5eafe000000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000500a50100768aa1e50751d31c7e16708903f6906621da68fd1daae12210480dac10d8a57b02018002ff2353719738df451db5eaa049f2b8c95493c34b008aa4d9d452e6820bec66034b43399405dfc64024a1b9ff974fb9b1f428461d28b7d262c16d1dc5d8894542f52764eed6c6f89f6a07781f035707aab471e846a9b815c73d0eb620cf345b82fdcdf56e93f5b4fe0f4892abe48971a5bb28205ff020f115aae2e4e5a6327309";

        // Create KeyPairs
        const keyPairA = KeyPair.fromHex(_keypair_a);
        const keyPairB = KeyPair.fromHex(_keypair_b);
        const keyPairC = KeyPair.fromHex(_keypair_c);

        // Generate the multi-signature wallet address
        const walletAddress = Address.fromPublicKeys(
            [keyPairA.publicKey, keyPairB.publicKey, keyPairC.publicKey],
            2,
        );
        expect(walletAddress.toUserFriendlyAddress()).toEqual(_wallet_address);

        // Create public key combinations for a 2-of-3 wallet
        const combinedPublicKeys = PublicKey.combinations(
            [keyPairA.publicKey, keyPairB.publicKey, keyPairC.publicKey],
            2,
        );

        // Create an unsigned transaction from the multi-signature wallet
        const tx = TransactionBuilder.newBasic(walletAddress, Address.NULL, BigInt(10), 0n, 0, 5);
        expect(tx.toHex()).toEqual(_unsigned_transaction);

        // Signer A and B will be signing, aggregate their public keys
        const aggregatedPublicKey = PublicKey.sum([keyPairA.publicKey, keyPairB.publicKey]);
        expect(aggregatedPublicKey.toHex()).toEqual(_signing_public_key);

        // Ensure the signing public key is part of the combinations
        expect(combinedPublicKeys.map(key => key.toHex()).includes(aggregatedPublicKey.toHex())).toBe(true);

        // Create commitment pairs for A and B
        const commitmentPairA1 = CommitmentPair.fromHex(_commitment_pair_a_1);
        const commitmentPairA2 = CommitmentPair.fromHex(_commitment_pair_a_2);

        const commitmentPairB1 = CommitmentPair.fromHex(_commitment_pair_b_1);
        const commitmentPairB2 = CommitmentPair.fromHex(_commitment_pair_b_2);

        // Aggregate commitments for MuSig2
        const aggregated_commitment = Commitment.sumMuSig2(
            [keyPairA.publicKey, keyPairB.publicKey],
            [
                [commitmentPairA1.commitment, commitmentPairA2.commitment],
                [commitmentPairB1.commitment, commitmentPairB2.commitment],
            ],
            tx.serializeContent()
        );
        expect(aggregated_commitment.toHex()).toEqual(_aggregate_commitment);

        // Create partial signatures
        const partialSigA = PartialSignature.create(
            keyPairA.privateKey,
            keyPairA.publicKey,
            [commitmentPairA1, commitmentPairA2],
            [keyPairB.publicKey],
            [[commitmentPairB1.commitment, commitmentPairB2.commitment]],
            tx.serializeContent(),
        );
        expect(partialSigA.toHex()).toEqual(_partial_signature_a);

        const partialSigB = PartialSignature.create(
            keyPairB.privateKey,
            keyPairB.publicKey,
            [commitmentPairB1, commitmentPairB2],
            [keyPairA.publicKey],
            [[commitmentPairA1.commitment, commitmentPairA2.commitment]],
            tx.serializeContent(),
        );
        expect(partialSigB.toHex()).toEqual(_partial_signature_b);

        // Aggregate partial signatures and convert to final signature
        const signature = PartialSignature
            .sum([partialSigA, partialSigB])
            .toSignature(aggregated_commitment);

        // Create and attach the multi-signature proof to the transaction
        const proof = SignatureProof.multiSig(aggregatedPublicKey, combinedPublicKeys, signature);
        tx.proof = proof.serialize();

        // Verify that everything worked correctly
        expect(tx.toHex()).toEqual(_signed_transaction);
        expect(() => tx.verify()).not.toThrow();
    });

    it('rejects empty public keys and commitment groups', () => {
        expect(() => Commitment.sumMuSig2([], [], new Uint8Array(0)))
            .toThrow('At least one public key must be provided');
    });

    it('rejects malformed commitment groups', () => {
        const signer = KeyPair.generate();
        const invalidGroup = [CommitmentPair.generate().commitment];

        expect(() => Commitment.sumMuSig2(
            [signer.publicKey],
            [invalidGroup],
            new Uint8Array(0),
        )).toThrow('Number of commitments in each group must be 2');
    });
});
