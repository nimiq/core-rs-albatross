use nimiq_collections::{SegmentTree};
use nimiq_collections::segment_tree::Range;

#[test]
fn it_works() {
    let tree = SegmentTree::<usize, usize>::new(&mut [
        (0, 1),
        (1, 3),
        (2, 4),
        (3, 1),
        (4, 2),
    ]);

    assert_eq!(
        Some(Range { weight: 4, offset: 4 }),
        tree.get(2)
    );

    assert_eq!(
        Some(2),
        tree.find(6)
    );
}
