use std::thread::sleep;
use std::time::Duration;

use nimiq_test_log::test;
use nimiq_utils::rate_limit::*;

#[test]
fn it_limits_access() {
    let mut limit = RateLimit::new_per_minute(3);

    assert_eq!(limit.num_allowed(), 3);
    assert!(limit.note_single());
    assert_eq!(limit.num_allowed(), 2);
    assert!(limit.note(2));
    assert_eq!(limit.num_allowed(), 0);
    assert!(!limit.note(1));
    assert_eq!(limit.num_allowed(), 0);
}

#[test]
fn it_frees_limit_after_time() {
    let time_period = Duration::from_millis(100);
    let mut limit = RateLimit::new(1, time_period);

    assert_eq!(limit.num_allowed(), 1);
    assert!(limit.note(1));
    assert_eq!(limit.num_allowed(), 0);
    assert!(!limit.note_single());

    sleep(time_period);

    assert_eq!(limit.num_allowed(), 1);
    assert!(limit.note(1));
}
