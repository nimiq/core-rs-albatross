import init, { WebClient, WebClientConfiguration } from "./pkg/nimiq_web_client.js";

init().then(async () => {
    const config = new WebClientConfiguration(["/dns4/seed1.v2.nimiq-testnet.com/tcp/8443/ws"], "debug");
    const client = await WebClient.create(config);
    client.subscribe_consensus();
    client.subscribe_blocks();
    client.subscribe_peers();
    // client.subscribe_statistics();
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
     * @param {'joined' | 'left'} type
     * @param {string} peerId
     * @param {number} numPeers
     */
    peer_listener(type, peerId, numPeers, peer_info) {
        if (peer_info == null) {
            console.log(`Peer ${type}: ${peerId} - now ${numPeers} peers connected`);
        } else {
            const address = peer_info.getAddress();
            const nodeType = peer_info.getNodeType();
            console.log(`${nodeType} peer ${type}: ${peerId}@${address} - now ${numPeers} peers connected`);
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
