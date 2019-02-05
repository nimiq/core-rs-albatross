use std::thread::sleep;
use std::time::Duration;

use nimiq_collections::queue::Queue;
use nimiq_utils::throttled_queue::*;

#[test]
fn it_can_enqueue_dequeue() {
    let mut queue = ThrottledQueue::new(1000, Duration::default(), 0, None);

    queue.enqueue(1);
    queue.enqueue(2);
    queue.enqueue(3);
    queue.enqueue(4);

    assert_eq!(queue.dequeue(), Some(1));
    assert_eq!(queue.dequeue_multi(2), vec![2, 3]);
    assert!(queue.is_available());
    assert_eq!(queue.num_available(), 1);
}

#[test]
fn it_can_throttle() {
    let interval = Duration::new(0, 50000000);
    let mut queue = ThrottledQueue::new(2, interval, 1, Some(10));

    queue.enqueue(1);
    queue.enqueue(2);
    queue.enqueue(3);
    queue.enqueue(4);
    queue.enqueue(5);
    queue.enqueue(6);

    // TODO: This test is dependent on timing!
    assert_eq!(queue.dequeue_multi(3), vec![1, 2]);
    sleep(interval);
    assert_eq!(queue.dequeue_multi(2), vec![3]);
    sleep(3 * interval);
    assert_eq!(queue.num_available(), 2);
}
