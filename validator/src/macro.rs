use std::pin::Pin;

use futures::task::{Context, Poll};
use futures::Future;

pub(crate) struct ProduceMacroBlock {
    //tendermint: BoxFuture<'static, Result<MacroBlock, ()>>,
}

impl ProduceMacroBlock {
    pub fn new() -> Self {
        unimplemented!()
    }
}

impl Future for ProduceMacroBlock {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        unimplemented!()
    }
}
