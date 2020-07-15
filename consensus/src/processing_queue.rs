use futures::stream::Stream;
use futures::sync::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::{future, Future};

pub trait Forward {
    type ItemType;
    fn forward(value: Self::ItemType);
}

pub struct ProcessingQueue<F: Forward> {
    consumer: Option<Consumer<F>>,
    producer: UnboundedSender<F::ItemType>,
}

impl<F: Forward> ProcessingQueue<F> {
    pub fn new(f: F) {
        let consumer = Consumer { f };
        let (tx, rx) = unbounded();
        ProcessingQueue {
            consumer: Some(rx.for_each(move |item| consumer.consume(item))),
            producer: tx,
        }
    }
}

struct Consumer<F: Forward> {
    f: F,
}

impl<F: Forward> Consumer<F> {
    fn consume(&self, item: F::ItemType) -> impl Future<Item = (), Error = ()> {
        self.f.forward(item);
        future::ok(())
    }
}
