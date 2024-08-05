const Nimiq = require("@nimiq/core");

async function main() {
    const config = new Nimiq.ClientConfiguration();
    // config.logLevel('debug');

    const client = await Nimiq.Client.create(config.build());

    let peerCount = 0;
    client.addPeerChangedListener((id, reason, count) => {
        peerCount = count;
    });

    let blockHeight = 0;
    client.addHeadChangedListener(async () => {
        const block = await client.getHeadBlock();
        blockHeight = block.height;
    })

    setInterval(async () => {
        const consensus = await client.isConsensusEstablished();
        console.log(`Consensus ${consensus ? 'established' : 'not established'} - Peers: ${peerCount}, Block height: ${blockHeight}`);
    }, 1000);
}

main();
