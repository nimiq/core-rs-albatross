import init, * as Nimiq from "./pkg/nimiq_web_client.js";

window.Nimiq = Nimiq;

init().then(async () => {
    const config = new Nimiq.ClientConfiguration();
    config.seedNodes(['/dns4/seed1.v2.nimiq-testnet.com/tcp/8443/ws']);
    config.logLevel('debug');

    const client = await config.instantiateClient();
    window.client = client; // Prevent garbage collection and for playing around

    client.addConsensusChangedListener(
        (state) => {
            console.log(`Consensus ${state.toUpperCase()}`);
        },
    );

    client.addHeadChangedListener(
        async (hash, reason, revertedBlocks, adoptedBlocks) => {
            const serializedBlock = await client.getBlock(hash);

            const { blockNumber, timestamp } = __unserializeBlock(serializedBlock);
            const rebranchLength = revertedBlocks.length;

            console.log([
                'Blockchain:',
                reason,
                ...(rebranchLength ? [rebranchLength] : []),
                'at',
                blockNumber,
                `(${new Date(timestamp).toISOString().substring(0, 19).replace('T', ' ')} UTC)`
            ].join(' '));
        },
    );

    client.addPeerChangedListener((peerId, reason, numPeers, peerInfo) => {
        if (peerInfo) {
            const host = peerInfo.address.split('/')[2];
            console.log(`Peer ${reason}: [${peerInfo.type}] ${peerId}@${host} - now ${numPeers} peers connected`);
        } else {
            console.log(`Peer ${reason}: ${peerId} - now ${numPeers} peers connected`);
        }
    });

    /**
     * @param {string} privateKey
     * @param {string} recipient
     * @param {number} amount
     * @param {string} [message]
     * @param {number} [fee]
     * @returns {Promise<string>}
     */
    window.sendBasicTransaction = async (privateKey, recipient, amount, message, fee = 0) => {
        if (!client.isConsensusEstablished()) {
            throw new Error('Consensus not yet established');
        }

        const keyPair = Nimiq.KeyPair.derive(Nimiq.PrivateKey.fromHex(privateKey));

        const transactionBuilder = client.transactionBuilder();

        /** @type {Nimiq.Transaction} */
        let transaction;
        if (message) {
            const messageBytes = new TextEncoder().encode(message);

            transaction = transactionBuilder.newBasicWithData(
                keyPair.toAddress(),
                Nimiq.Address.fromString(recipient),
                messageBytes,
                BigInt(amount),
                BigInt(fee),
            ).sign(keyPair);
        } else {
            transaction = transactionBuilder.newBasic(
                keyPair.toAddress(),
                Nimiq.Address.fromString(recipient),
                BigInt(amount),
                BigInt(fee),
            ).sign(keyPair);
        }

        return client.sendTransaction(transaction);
    }

    /**
     * @param {string} privateKey
     * @param {string} delegation
     * @param {number} amount
     * @param {number} [fee]
     * @returns {Promise<string>}
     */
    window.sendCreateStakerTransaction = async (privateKey, delegation, amount, fee = 0) => {
        if (!client.isConsensusEstablished()) {
            throw new Error('Consensus not yet established');
        }

        const keyPair = Nimiq.KeyPair.derive(Nimiq.PrivateKey.fromHex(privateKey));

        const transactionBuilder = client.transactionBuilder();

        const transaction = transactionBuilder.newCreateStaker(
            keyPair.toAddress(),
            Nimiq.Address.fromString(delegation),
            BigInt(amount),
            BigInt(fee),
        ).sign(keyPair);

        return client.sendTransaction(transaction);
    }

    /**
     * @param {string} privateKey
     * @param {string} newDelegation
     * @param {number} [fee]
     * @returns {Promise<string>}
     */
    window.sendUpdateStakerTransaction = async (privateKey, newDelegation, fee = 0) => {
        if (!client.isConsensusEstablished()) {
            throw new Error('Consensus not yet established');
        }

        const keyPair = Nimiq.KeyPair.derive(Nimiq.PrivateKey.fromHex(privateKey));

        const transactionBuilder = client.transactionBuilder();

        const transaction = transactionBuilder.newUpdateStaker(
            keyPair.toAddress(),
            Nimiq.Address.fromString(newDelegation),
            BigInt(fee),
        ).sign(keyPair);

        return client.sendTransaction(transaction);
    }

    /**
     * @param {string} privateKey
     * @param {number} amount
     * @param {number} [fee]
     * @returns {Promise<string>}
     */
    window.sendUnstakeTransaction = async (privateKey, amount, fee = 0) => {
        if (!client.isConsensusEstablished()) {
            throw new Error('Consensus not yet established');
        }

        const keyPair = Nimiq.KeyPair.derive(Nimiq.PrivateKey.fromHex(privateKey));

        const transactionBuilder = client.transactionBuilder();

        const transaction = transactionBuilder.newUnstake(
            keyPair.toAddress(),
            BigInt(amount),
            BigInt(fee),
        ).sign(keyPair);

        return client.sendTransaction(transaction);
    }
});

/**
 * @param {Uint8Array} serializedBlock
 */
function __unserializeBlock(serializedBlock) {
    // Rudimentary block parsing - TODO: Properly deserialize the whole light block

    /** @type {Uint8Array} */
    let blockNumberBytes;

    /** @type {Uint8Array} */
    let timestampBytes;

    const blockType = serializedBlock[0];
    if (blockType === 1) { // Macro block
        const _version = serializedBlock.subarray(1, 1 + 2); // u16
        blockNumberBytes = serializedBlock.subarray(3, 3 + 4); // u32
        const _round = serializedBlock.subarray(7, 7 + 4); // u32
        timestampBytes = serializedBlock.subarray(11, 11 + 8); // u64
    } else if (blockType === 2) { // Micro block
        const _version = serializedBlock.subarray(1, 1 + 2); // u16
        blockNumberBytes = serializedBlock.subarray(3, 3 + 4); // u32
        timestampBytes = serializedBlock.subarray(7, 7 + 8); // u64
    } else {
        throw new Error(`Invalid block type: ${blockType}`);
    }

    const blockNumber = new Uint32Array(new Uint8Array(blockNumberBytes).reverse().buffer)[0];

    const timestampBig = new BigUint64Array(new Uint8Array(timestampBytes).reverse().buffer)[0];
    const timestamp = parseInt(timestampBig.toString(10));

    return {
        blockNumber,
        timestamp,
    };
}
