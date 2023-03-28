import init, * as Nimiq from "./pkg/nimiq_web_client.js";

window.Nimiq = Nimiq;

init().then(async () => {
    const config = new Nimiq.ClientConfiguration();
    config.seedNodes(['/dns4/seed1.pos.nimiq-testnet.com/tcp/8443/wss']);
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
            const block = await client.getBlock(hash);
            const rebranchLength = revertedBlocks.length;

            console.log([
                'Blockchain:',
                reason,
                ...(rebranchLength ? [rebranchLength] : []),
                'at',
                block.height,
                `(${new Date(block.timestamp).toISOString().substring(0, 19).replace('T', ' ')} UTC)`
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
