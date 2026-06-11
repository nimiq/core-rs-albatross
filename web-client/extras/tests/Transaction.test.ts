import {
    AccountType,
    Address,
    KeyPair,
    type PlainTransactionDetails,
    PrivateKey,
    PublicKey,
    Transaction,
    TransactionFormat,
} from '@nimiq/core';
import { describe, expect, it } from 'vitest';

describe('Transaction', () => {
    it('has the correct types for format and sender/recipient type', () => {
        const sender = PrivateKey.fromHex('41771e617f4a847642f9c0cd0fdb9b22b0c9dfdd2548fe7a6cad0a782e588f63');
        const recipient = Address.fromUserFriendlyAddress('NQ72 9DS3 CQHA G44E ND6G DCQT 561M 3G2A H9FL');

        const transaction = new Transaction(
            PublicKey.derive(sender).toAddress(),
            AccountType.Basic,
            new Uint8Array(0),
            recipient,
            AccountType.Basic,
            new Uint8Array(0),
            100_00000n,
            0n,
            0,
            1,
            6,
        );
        transaction.sign(KeyPair.derive(sender));

        // Verify that the enums are numbers
        expect(AccountType.Basic).toBe(0);
        expect(TransactionFormat.Basic).toBe(0);

        // Now check the transaction fields
        expect(transaction.format).toBe(TransactionFormat.Basic);
        expect(transaction.senderType).toBe(AccountType.Basic);
        expect(transaction.recipientType).toBe(AccountType.Basic);

        // In plain format, the fields should now be strings
        const plain = transaction.toPlain();
        expect(plain.format).toBe('basic');
        expect(plain.senderType).toBe('basic');
        expect(plain.recipientType).toBe('basic');
    });

    it('can deserialize PoS style plain transaction details', () => {
        const plain = {
            'transactionHash': '9bf8872a4d8a1132c313e26b35bcf936e2e1ec07b2ee4716b487a2d545019483',
            'format': 'extended',
            'sender': 'NQ71 4NP1 51M7 3M05 BGAS VPTH JRY3 RADA ALVQ',
            'senderType': 'basic',
            'recipient': 'NQ13 4NR7 212D SACQ 01FQ L5DK Q2T9 XJ47 DRTP',
            'recipientType': 'basic',
            'value': 10000000,
            'fee': 0,
            'feePerByte': 0,
            'validityStartHeight': 3592850,
            'network': 'mainalbatross',
            'flags': 0,
            'senderData': {
                'type': 'raw',
                'raw': '',
            },
            'data': {
                'type': 'raw',
                'raw': '0082809287',
            },
            'proof': {
                'type': 'standard',
                'raw':
                    '004490c883a95fc7c44e45043108436973b071851ed2a203111ddb2f088056f0fc00954fd27956cfb4292cd28e8da6565dd8a81e7cc7a7625659f37ccc6def4bbdac3029765538db0cb5edb3f0685e4cb1d569329dd5bf7899bdd03bf096398be50c',
                'signature':
                    '954fd27956cfb4292cd28e8da6565dd8a81e7cc7a7625659f37ccc6def4bbdac3029765538db0cb5edb3f0685e4cb1d569329dd5bf7899bdd03bf096398be50c',
                'publicKey': '4490c883a95fc7c44e45043108436973b071851ed2a203111ddb2f088056f0fc',
                'signer': 'NQ71 4NP1 51M7 3M05 BGAS VPTH JRY3 RADA ALVQ',
                'pathLength': 0,
            },
            'size': 171,
            'state': 'confirmed',
            'executionResult': true,
            'blockHeight': 3592909,
            'confirmations': 53914,
            'timestamp': 1732177058489,
            'fiatValue': {
                'usd': 0.2958,
            },
        };

        let tx: Transaction | undefined;
        expect(() => tx = Transaction.fromPlain(plain as PlainTransactionDetails)).not.toThrow();
        if (!tx) throw new Error('Transaction.fromPlain returned undefined');
        const plain2 = tx.toPlain();
        expect(plain2.proof).toEqual(plain.proof);
        expect(plain2.size).toEqual(plain.size);
    });

    it('can deserialize PoW style plain transaction details', () => {
        // PoW plain transactions don't have `senderData` and their `data` and `proof` fields have no `type` property
        const plain = {
            'transactionHash': 'e5ccd2e7b892474b29af0d9fdc5a802989a8e48d77fb6fe85966a3f8fb65049e',
            'format': 'extended',
            'sender': 'NQ71 4NP1 51M7 3M05 BGAS VPTH JRY3 RADA ALVQ',
            'senderType': 'basic',
            'recipient': 'NQ26 TT94 M4LL F8QS UH4M XPQK 9P42 QNH9 XVNY',
            'recipientType': 'basic',
            'value': 10,
            'fee': 0,
            'feePerByte': 0,
            'validityStartHeight': 3388462,
            'network': 'main',
            'flags': 0,
            'data': {
                'raw': '4e6f7720676f20666f72746820616e6420726567697374657220796f757273656c6621',
            },
            'proof': {
                'signature':
                    '98be457841e99d998cc54e96306a82e106bc5306271cd2194eba1d8e7653baff6e5e3b3344c11da401d4f42ab5763e4f3147662f07ac4c32addd3e15da469707',
                'publicKey': '4490c883a95fc7c44e45043108436973b071851ed2a203111ddb2f088056f0fc',
                'signer': 'NQ71 4NP1 51M7 3M05 BGAS VPTH JRY3 RADA ALVQ',
                'pathLength': 0,
                'raw':
                    '4490c883a95fc7c44e45043108436973b071851ed2a203111ddb2f088056f0fc0098be457841e99d998cc54e96306a82e106bc5306271cd2194eba1d8e7653baff6e5e3b3344c11da401d4f42ab5763e4f3147662f07ac4c32addd3e15da469707',
            },
            'size': 200,
            'state': 'confirmed',
            'blockHash': 'f8a0d2a34352368bbb07590434b58f1c23c31b12b8b6368eb92cb84928968575',
            'blockHeight': 3388464,
            'confirmations': 29911,
            'timestamp': 1727958157,
            'fiatValue': {
                'eur': 9.725e-8,
                'usd': 1.0729533333333334e-7,
            },
        };

        let tx: Transaction | undefined;
        expect(() => tx = Transaction.fromPlain(plain as unknown as PlainTransactionDetails)).not.toThrow();
        if (!tx) throw new Error('Transaction.fromPlain returned undefined');

        const plain2 = tx.toPlain();
        // Cannot compare proof field yet, as re-serialization creates a PoS proof with a a leading 0x00-byte
        // expect(plain2.proof).toEqual(plain.proof);
        expect(plain2.size).toEqual(plain.size);
    });
});
