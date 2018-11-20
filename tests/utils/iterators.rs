use nimiq::utils::iterators::*;

#[test]
fn it_can_iterate_over_two_iterators() {
    let a = vec![1, 3, 5, 7, 9];
    let b = vec![2, 4, 6, 8, 10];

    let combined: Vec<i32> = Alternate::new(a.iter(), b.iter()).map(|&i| i).collect();
    assert_eq!(combined, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10], "alternate iterator did not iterate correctly");
}

#[test]
fn it_can_determine_the_correct_size() {
    let a = vec![1, 3, 5, 7, 9];
    let b = vec![2, 4, 6, 8, 10];

    let it = Alternate::new(a.iter(), b.iter());
    assert_eq!(it.size_hint(), (10, Some(10)), "alternate iterator did not determine the correct size hint");
}

#[test]
fn it_can_determine_the_correct_count() {
    let a = vec![1, 3, 5, 7, 9];
    let b = vec![2, 4, 6, 8, 10];

    let it = Alternate::new(a.iter(), b.iter());
    assert_eq!(it.count(), 10, "alternate iterator did not determine the correct count");

    let it = Alternate::new(a.iter(), b.iter());
    assert_eq!(it.skip(1).count(), 9, "alternate iterator did not determine the correct count after skipping one item");
    let it = Alternate::new(a.iter(), b.iter());
    assert_eq!(it.skip(2).count(), 8, "alternate iterator did not determine the correct count after skipping two items");
}
