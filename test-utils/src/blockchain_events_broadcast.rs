use std::fmt::Debug;

use tokio::sync::broadcast::{error::TryRecvError, Receiver};

pub fn try_collect_events<T: Clone>(receiver: &mut Receiver<T>) -> Result<Vec<T>, TryRecvError> {
    let mut events = Vec::new();

    loop {
        match receiver.try_recv() {
            Ok(event) => events.push(event),
            Err(TryRecvError::Empty) => return Ok(events),
            Err(error) => return Err(error),
        }
    }
}

pub fn assert_events_eq_unordered<T>(mut actual: Vec<T>, expected: Vec<T>)
where
    T: Debug + PartialEq,
{
    assert_eq!(actual.len(), expected.len(), "event count mismatch");

    for expected_event in expected {
        let index = actual
            .iter()
            .position(|event| event == &expected_event)
            .unwrap_or_else(|| panic!("missing event: {expected_event:?}, actual: {actual:?}"));

        actual.swap_remove(index);
    }
}
