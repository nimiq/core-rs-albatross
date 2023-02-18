import init, * as Nimiq from "./pkg/nimiq_web_client.js";

window.Nimiq = Nimiq;

init().then(async () => {
    const config = new Nimiq.ClientConfiguration();
    config.seedNodes(['/dns4/seed1.v2.nimiq-testnet.com/tcp/8443/ws']);
    config.logLevel('debug');

    const client = await config.instantiateClient();
    window.client = client; // Prevent garbage collection and for playing around

    client.subscribe_consensus();
    client.subscribe_blocks();
    client.subscribe_peers();
    // client.subscribe_statistics();

    /**
     * @param {string} privateKey
     * @param {string} recipient
     * @param {number} amount
     * @param {number} [fee]
     * @returns {Promise<string>}
     */
    window.sendTransaction = async (privateKey, recipient, amount, fee = 0) => {
        if (!client.isEstablished()) {
            throw new Error('Consensus not yet established');
        }

        const keyPair = Nimiq.KeyPair.derive(Nimiq.PrivateKey.fromHex(privateKey));

        const transaction = Nimiq.Transaction.newBasicTransaction(
            keyPair.toAddress(),
            Nimiq.Address.fromString(recipient),
            BigInt(amount),
            BigInt(fee),
            client.blockNumber(),
            client.networkId,
        );

        keyPair.signTransaction(transaction);

        await client.sendTransaction(transaction);
        return transaction.hash();
    }
});

window.__wasm_imports = {
    /**
     * @param {boolean} established
     */
    consensus_listener(established) {
        console.log(`Consensus: ${established ? 'established =)' : 'lost =('}`);
    },

    /**
     * @param {string} type
     * @param {Uint8Array} serializedBlock
     * @param {number?} rebranchLength
     */
    block_listener(type, serializedBlock, rebranchLength) {
        // Rudimentary block parsing - TODO: Properly deserialize the whole light block

        /** @type {Uint8Array} */
        let blockNumberBytes;

        /** @type {Uint8Array} */
        let timestampBytes;

        const blockType = serializedBlock[0];
        if (blockType === 1) {
            // Macro block
            const _version = serializedBlock.subarray(1, 1 + 2); // u16
            blockNumberBytes = serializedBlock.subarray(3, 3 + 4); // u32
            const _round = serializedBlock.subarray(7, 7 + 4); // u32
            timestampBytes = serializedBlock.subarray(11, 11 + 8); // u64
        } else if (blockType === 2) {
            // Micro block
            const _version = serializedBlock.subarray(1, 1 + 2); // u16
            blockNumberBytes = serializedBlock.subarray(3, 3 + 4); // u32
            timestampBytes = serializedBlock.subarray(7, 7 + 8); // u64
        } else {
            throw new Error(`Invalid block type: ${blockType}`);
        }

        const blockNumber = new Uint32Array(new Uint8Array(blockNumberBytes).reverse().buffer)[0];
        const timestampBig = new BigUint64Array(new Uint8Array(timestampBytes).reverse().buffer)[0];
        const timestamp = parseInt(timestampBig.toString(10));

        console.log([
            'Blockchain:',
            type,
            ...(rebranchLength ? [rebranchLength] : []),
            'at',
            blockNumber,
            `(${new Date(timestamp).toISOString().substring(0, 19).replace('T', ' ')} UTC)`
        ].join(' '));
    },

    /**
     * @param {'joined' | 'left'} eventType
     * @param {string} peerId
     * @param {number} numPeers
     * @param {{ type: string, address: string } | null} peerInfo
     */
    peer_listener(eventType, peerId, numPeers, peerInfo) {
        if (peerInfo) {
            const host = peerInfo.address.split('/')[2];
            console.log(`Peer ${eventType}: [${peerInfo.type}] ${peerId}@${host} - now ${numPeers} peers connected`);
        } else {
            console.log(`Peer ${eventType}: ${peerId} - now ${numPeers} peers connected`);
        }
    },

    /**
     * @param {boolean} established
     * @param {number} blockNumber
     * @param {number} numPeers
     */
    statistics_listener(established, blockNumber, numPeers) {
        console.log({ established, blockNumber, numPeers });
    },
}
