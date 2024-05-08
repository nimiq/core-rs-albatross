use std::task::{Context, Waker};

pub trait WakerExt {
    fn store_waker(&mut self, context: &Context);
}

impl WakerExt for Option<Waker> {
    fn store_waker(&mut self, context: &Context) {
        let waker = context.waker();
        match self {
            Some(stored) => stored.clone_from(waker),
            None => *self = Some(waker.clone()),
        }
    }
}
