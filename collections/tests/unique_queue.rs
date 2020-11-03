use nimiq_collections::{Queue, UniqueLinkedList};

#[test]
fn it_can_correctly_dequeue_elements() {
    let mut q = UniqueLinkedList::new();

    assert_eq!(q.len(), 0);

    q.enqueue(3);
    q.enqueue(1);
    q.enqueue(2);

    assert_eq!(q.len(), 3);
    assert_eq!(q.dequeue(), Some(3));
    assert_eq!(q.len(), 2);
    assert_eq!(q.dequeue(), Some(1));
    assert_eq!(q.len(), 1);
    assert_eq!(q.dequeue(), Some(2));
    assert_eq!(q.len(), 0);
}

#[test]
fn it_can_clear_itself() {
    let mut q = UniqueLinkedList::new();

    assert_eq!(q.len(), 0);

    q.enqueue(3);
    q.enqueue(1);
    q.enqueue(2);

    assert_eq!(q.len(), 3);
    q.clear();
    assert_eq!(q.len(), 0);
    assert_eq!(q.dequeue(), None);
    assert_eq!(q.is_empty(), true);
}

#[test]
fn it_can_peek() {
    let mut q = UniqueLinkedList::new();

    q.enqueue(3);
    q.enqueue(1);

    assert_eq!(q.peek(), Some(&3));
    q.dequeue();
    assert_eq!(q.peek(), Some(&1));
    q.dequeue();
    assert_eq!(q.peek(), None);
}

#[test]
fn it_can_dequeue_multi() {
    let mut q = UniqueLinkedList::new();

    q.enqueue(3);
    q.enqueue(1);
    q.enqueue(2);

    assert_eq!(q.dequeue_multi(2), vec![3, 1]);
    assert_eq!(q.dequeue_multi(2), vec![2]);
    assert_eq!(q.dequeue_multi(2), Vec::<i32>::new());
}

#[test]
fn it_can_enqueue_unique() {
    let mut q = UniqueLinkedList::new();

    q.enqueue(3);
    q.enqueue(1);
    q.enqueue(2);
    q.enqueue(3);
    q.enqueue(2);

    assert_eq!(q.len(), 3);
    assert_eq!(q.dequeue(), Some(3));
    assert_eq!(q.dequeue(), Some(1));
    assert_eq!(q.dequeue(), Some(2));
}

#[test]
fn it_can_append() {
    let mut q = UniqueLinkedList::new();

    let mut q1 = UniqueLinkedList::new();
    q1.extend(vec![3, 1, 2]);

    q.append(&mut q1);
    q.extend(vec![3, 2, 4]);

    assert_eq!(q.len(), 4);
    assert_eq!(q.dequeue(), Some(3));
    assert_eq!(q.dequeue(), Some(1));
    assert_eq!(q.dequeue(), Some(2));
    assert_eq!(q.dequeue(), Some(4));
}

#[test]
fn it_can_append_2() {
    let mut q = UniqueLinkedList::new();

    let mut q1 = UniqueLinkedList::new();
    q1.extend(vec![3, 1, 2]);

    q.append(&mut q1);
    q.dequeue();
    q.extend(vec![3, 2, 4]);

    assert_eq!(q.len(), 4);
    assert_eq!(q.dequeue(), Some(1));
    assert_eq!(q.dequeue(), Some(2));
    assert_eq!(q.dequeue(), Some(3));
    assert_eq!(q.dequeue(), Some(4));
}

#[test]
fn it_can_extend_3() {
    let mut q = UniqueLinkedList::new();

    q.extend(vec![3, 1, 3, 1, 3]);
    q.dequeue();
    q.dequeue();
    q.extend(vec![3, 4, 1]);

    assert_eq!(q.len(), 3);
    assert_eq!(q.dequeue(), Some(3));
    assert_eq!(q.dequeue(), Some(4));
    assert_eq!(q.dequeue(), Some(1));
}

#[test]
fn it_can_extend_4() {
    let mut q = UniqueLinkedList::new();

    q.extend(vec![3, 1, 3, 1, 3]);
    q.dequeue();
    q.extend(vec![3, 4, 1]);

    assert_eq!(q.len(), 3);
    assert_eq!(q.dequeue(), Some(1));
    assert_eq!(q.dequeue(), Some(3));
    assert_eq!(q.dequeue(), Some(4));
}

#[test]
fn it_can_extend_5() {
    let mut q = UniqueLinkedList::new();

    q.extend(vec![3, 1, 3, 1, 3]);
    q.dequeue_multi(2);
    q.extend(vec![3, 4, 1]);

    assert_eq!(q.len(), 3);
    assert_eq!(q.dequeue(), Some(3));
    assert_eq!(q.dequeue(), Some(4));
    assert_eq!(q.dequeue(), Some(1));
}

#[test]
fn it_can_remove() {
    let mut q = UniqueLinkedList::new();
    q.extend(vec![3, 1, 2, 4, 5]);
    assert_eq!(q.remove(&2), Some(2));
    assert_eq!(q.remove(&3), Some(3));
    assert_eq!(q.remove(&5), Some(5));
    assert_eq!(q.remove(&9), None);
    assert_eq!(q.len(), 2);

    assert_eq!(q.dequeue(), Some(1));
    assert_eq!(q.dequeue(), Some(4));
}

#[test]
fn it_can_test_contains() {
    let mut q = UniqueLinkedList::new();
    q.extend(vec![3, 1, 2, 4, 5]);

    assert_eq!(q.contains(&1), true);
    assert_eq!(q.contains(&8), false);
}

#[test]
fn it_can_requeue() {
    let mut q = UniqueLinkedList::new();

    q.requeue(3);
    q.requeue(1);
    q.requeue(2);
    q.requeue(3);
    q.requeue(2);

    assert_eq!(q.len(), 3);
    assert_eq!(q.dequeue(), Some(1));
    assert_eq!(q.dequeue(), Some(3));
    assert_eq!(q.dequeue(), Some(2));
}
