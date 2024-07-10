const Nimiq = require("@nimiq/core");

async function main() {
    const config = new Nimiq.ClientConfiguration();
    // config.logLevel('debug');

    const client = await Nimiq.Client.create(config.build());

    let peerCount = 0;
    client.addPeerChangedListener((_peerId, _reason, newPeerCount) => {
        peerCount = newPeerCount;
    })

    setInterval(async () => {
        const consensus = await client.isConsensusEstablished();
        console.log(`Consensus ${consensus ? 'established' : 'not established'}, ${peerCount} peers`);
    }, 1000);
}

main();
