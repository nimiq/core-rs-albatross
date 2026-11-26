// A minimal consumer of the published @nimiq/core types, typechecked with the tsconfig.json in this directory.
// Run `yarn test:types` after building the types and the lib (see scripts/build.sh).

// The main entry (resolves to types/bundler.d.ts).
import type { Client, PlainClientConfiguration } from '@nimiq/core';
// The web entry (resolves to types/web.d.ts).
import init, { Address, BufferUtils, CryptoUtils, KeyPair, MnemonicUtils, SerialBuffer } from '@nimiq/core/web';

export async function main(createClient: (config: PlainClientConfiguration) => Promise<Client>): Promise<void> {
    await init();

    const client = await createClient({ networkId: 'TestAlbatross' });
    console.log(await client.getHeadHeight());

    const keyPair = KeyPair.generate();
    const address: Address = keyPair.toAddress();
    console.log(address.toUserFriendlyAddress());
    // The generated classes implement `[Symbol.dispose]()`, which must typecheck for consumers both with and without
    // the `esnext.disposable` lib (see `lib` in tsconfig.json and tsconfig.esnext.json).
    keyPair.free();

    const mnemonic = MnemonicUtils.entropyToMnemonic(CryptoUtils.getRandomValues(32));
    console.log(MnemonicUtils.mnemonicToEntropy(mnemonic).toHex());

    // SerialBuffer extends Uint8Array; its overrides must be valid for the consumer's TypeScript version.
    const buffer = new SerialBuffer(8);
    buffer.writeUint32(42);
    const view: Uint8Array = buffer.subarray(0, 4);
    const bytes: Uint8Array = buffer.read(4);
    console.log(view, bytes);

    // ArrayBuffer-backed byte arrays returned by the lib must be accepted by WebCrypto (`BufferSource`).
    await crypto.subtle.digest('SHA-256', buffer);
    await crypto.subtle.digest('SHA-256', buffer.subarray(0, 4));
    await crypto.subtle.digest('SHA-256', buffer.read(0));
    await crypto.subtle.digest('SHA-256', BufferUtils.fromUtf8('nimiq'));
    await crypto.subtle.digest('SHA-256', BufferUtils.fromHex('00ff'));
    await crypto.subtle.digest('SHA-256', BufferUtils.xor(view, bytes));
}
