use hash::{Blake2bHash, Hash};
use nimiq::consensus::networks::*;
use hex;

#[test]
fn it_has_expected_main_hash() {
    assert_eq!(
        get_network_info(NetworkId::Main).unwrap().genesis_block.header.hash::<Blake2bHash>().as_bytes(),
        &hex::decode("264AAF8A4F9828A76C550635DA078EB466306A189FCC03710BEE9F649C869D12").unwrap()[..]
    )
}
