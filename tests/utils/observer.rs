use nimiq::utils::observer::*;
use std::rc::Rc;
use std::cell::RefCell;

#[test]
fn it_can_register_notify_deregister() {
    let mut notifier: Notifier<u32> = Notifier::new();

    let event1_rc1 = Rc::new(RefCell::new(0));
    let event1_rc2 = event1_rc1.clone();
    let handle1 = notifier.register(move |e: u32| *event1_rc2.borrow_mut() = e);
    assert_eq!(*event1_rc1.borrow(), 0);

    let event2_rc1 = Rc::new(RefCell::new(0));
    let event2_rc2 = event2_rc1.clone();
    let handle2 = notifier.register(move |e: u32| *event2_rc2.borrow_mut() = e);
    assert_eq!(*event2_rc1.borrow(), 0);

    notifier.notify(42);
    assert_eq!(*event1_rc1.borrow(), 42);
    assert_eq!(*event2_rc1.borrow(), 42);

    notifier.notify(69);
    assert_eq!(*event1_rc1.borrow(), 69);
    assert_eq!(*event2_rc1.borrow(), 69);

    notifier.deregister(handle1);

    notifier.notify(4711);
    assert_eq!(*event1_rc1.borrow(), 69);
    assert_eq!(*event2_rc1.borrow(), 4711);

    notifier.deregister(handle2);

    notifier.notify(815);
    assert_eq!(*event1_rc1.borrow(), 69);
    assert_eq!(*event2_rc1.borrow(), 4711);

    // noop
    notifier.deregister(handle2);

    let event3_rc1 = Rc::new(RefCell::new(0));
    let event3_rc2 = event3_rc1.clone();
    let handle3 = notifier.register(move |e: u32| *event3_rc2.borrow_mut() = e);
    assert_eq!(*event3_rc1.borrow(), 0);

    let event4_rc1 = Rc::new(RefCell::new(0));
    let event4_rc2 = event4_rc1.clone();
    let handle4 = notifier.register(move |e: u32| *event4_rc2.borrow_mut() = e);
    assert_eq!(*event4_rc1.borrow(), 0);

    notifier.notify(5555);
    assert_eq!(*event1_rc1.borrow(), 69);
    assert_eq!(*event2_rc1.borrow(), 4711);
    assert_eq!(*event3_rc1.borrow(), 5555);
    assert_eq!(*event4_rc1.borrow(), 5555);

    notifier.deregister(handle3);

    notifier.notify(11);
    assert_eq!(*event1_rc1.borrow(), 69);
    assert_eq!(*event2_rc1.borrow(), 4711);
    assert_eq!(*event3_rc1.borrow(), 5555);
    assert_eq!(*event4_rc1.borrow(), 11);

    let event5_rc1 = Rc::new(RefCell::new(0));
    let event5_rc2 = event5_rc1.clone();
    let handle5 = notifier.register(move |e: u32| *event5_rc2.borrow_mut() = e);
    assert_eq!(*event5_rc1.borrow(), 0);

    notifier.notify(999);
    assert_eq!(*event1_rc1.borrow(), 69);
    assert_eq!(*event2_rc1.borrow(), 4711);
    assert_eq!(*event3_rc1.borrow(), 5555);
    assert_eq!(*event4_rc1.borrow(), 999);
    assert_eq!(*event5_rc1.borrow(), 999);

    notifier.deregister(handle5);

    notifier.notify(6666);
    assert_eq!(*event1_rc1.borrow(), 69);
    assert_eq!(*event2_rc1.borrow(), 4711);
    assert_eq!(*event3_rc1.borrow(), 5555);
    assert_eq!(*event4_rc1.borrow(), 6666);
    assert_eq!(*event5_rc1.borrow(), 999);
}
