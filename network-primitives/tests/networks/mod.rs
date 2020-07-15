use hex;

use nimiq_block::Block;
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_network_primitives::networks::*;

#[test]
fn it_has_expected_main_hash() {
    assert_eq!(
        NetworkInfo::from_network_id(NetworkId::Main)
            .genesis_block::<Block>()
            .header
            .hash::<Blake2bHash>()
            .as_bytes(),
        &hex::decode("264AAF8A4F9828A76C550635DA078EB466306A189FCC03710BEE9F649C869D12").unwrap()[..]
    )
}
