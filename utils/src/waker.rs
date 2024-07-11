use std::task::{Context, Waker};

pub trait WakerExt {
    fn store_waker(&mut self, context: &Context);
    fn wake(&mut self);
}

impl WakerExt for Option<Waker> {
    fn store_waker(&mut self, context: &Context) {
        let waker = context.waker();
        match self {
            Some(stored) => stored.clone_from(waker),
            None => *self = Some(waker.clone()),
        }
    }
    fn wake(&mut self) {
        if let Some(waker) = self.take() {
            waker.wake();
        }
    }
}
