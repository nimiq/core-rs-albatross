use nimiq_block::{MacroBlock, MacroHeader};
use nimiq_hash::Blake2bHash;
use nimiq_primitives::policy::Policy;

#[test]
fn test_next_interlink() {
    fn create_interlink_macro_block(election_number: u32, interlink: &[Blake2bHash]) -> MacroBlock {
        MacroBlock {
            header: MacroHeader {
                block_number: Policy::blocks_per_epoch() * election_number,
                interlink: Some(interlink.to_vec()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    let block2 = create_interlink_macro_block(2, &vec![]);
    let block3_interlink = vec![block2.hash()];
    assert_eq!(block2.get_next_interlink().unwrap(), block3_interlink);
    let block3 = create_interlink_macro_block(3, &block3_interlink);
    assert_eq!(block3.get_next_interlink().unwrap(), block3_interlink);
    let block4 = create_interlink_macro_block(4, &block3_interlink);
    let block5_interlink = vec![block4.hash(), block4.hash()];
    assert_eq!(block4.get_next_interlink().unwrap(), block5_interlink);
    let block5 = create_interlink_macro_block(5, &block5_interlink);
    assert_eq!(block5.get_next_interlink().unwrap(), block5_interlink);
    let block6 = create_interlink_macro_block(6, &block5_interlink);
    let block7_interlink = vec![block6.hash(), block4.hash()];
    assert_eq!(block6.get_next_interlink().unwrap(), block7_interlink);
    let block7 = create_interlink_macro_block(7, &block7_interlink);
    assert_eq!(block7.get_next_interlink().unwrap(), block7_interlink);
    let block8 = create_interlink_macro_block(8, &block7_interlink);
    let block9_interlink = vec![block8.hash(), block8.hash(), block8.hash()];
    assert_eq!(block8.get_next_interlink().unwrap(), block9_interlink);
    let block9 = create_interlink_macro_block(9, &block9_interlink);
    assert_eq!(block9.get_next_interlink().unwrap(), block9_interlink);
    let block10 = create_interlink_macro_block(10, &block9_interlink);
    let block11_interlink = vec![block10.hash(), block8.hash(), block8.hash()];
    assert_eq!(block10.get_next_interlink().unwrap(), block11_interlink);
}
