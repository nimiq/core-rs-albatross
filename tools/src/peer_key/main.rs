use beserial::Serialize;
use nimiq_network_libp2p::Keypair;

fn main() {
    let keypair = Keypair::generate_ed25519();

    let mut writer = Vec::<u8>::new();
    let _ = keypair.serialize(&mut writer);

    println!("PeerId: {}", keypair.public().into_peer_id().to_base58());
    println!("PeerKey: {}", hex::encode(writer));
}
