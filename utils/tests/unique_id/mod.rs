use nimiq_utils::unique_id::UniqueId;

#[test]
fn it_can_generate_unique_ids() {
    let id1 = UniqueId::new();
    let id2 = UniqueId::new();
    assert_ne!(id1, id2);

    let id1_copy = id1;
    let id2_copy = id2;
    assert_eq!(id1, id1_copy);
    assert_eq!(id2, id2_copy);
    assert_ne!(id1_copy, id2_copy);
}
