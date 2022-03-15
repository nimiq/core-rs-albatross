use std::pin::Pin;
use std::sync::{Arc, Weak};
use std::task::{Context, Poll};

use futures::stream::Stream;
use parking_lot::RwLock;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

pub type ListenerHandle = usize;

pub trait Listener<E>: Send + Sync {
    fn on_event(&self, event: &E);
}

impl<E, F: Fn(&E)> Listener<E> for F
where
    F: Send + Sync,
{
    fn on_event(&self, event: &E) {
        self(event);
    }
}

#[derive(Default)]
pub struct NotifierState<E> {
    listeners: Vec<(ListenerHandle, Box<dyn Listener<E> + 'static>)>,
    next_handle: ListenerHandle,
}

impl<E> NotifierState<E> {
    pub fn new() -> Self {
        Self {
            listeners: Vec::new(),
            next_handle: 0,
        }
    }

    pub fn register<T: Listener<E> + 'static>(&mut self, listener: T) -> ListenerHandle {
        let handle = self.next_handle;
        self.listeners.push((handle, Box::new(listener)));
        self.next_handle += 1;
        handle
    }

    pub fn deregister(&mut self, handle: ListenerHandle) {
        for (i, (stored_handle, _)) in self.listeners.iter().enumerate() {
            if handle == *stored_handle {
                self.listeners.remove(i);
                return;
            }
        }
    }

    pub fn notify(&self, event: E) {
        for (_, listener) in &self.listeners {
            listener.on_event(&event);
        }
    }
}

pub struct NotifierStream<E> {
    stream: UnboundedReceiverStream<E>,
    handle: ListenerHandle,
    state: Arc<RwLock<NotifierState<E>>>,
}

impl<E> NotifierStream<E> {
    pub fn new(
        stream: UnboundedReceiverStream<E>,
        handle: ListenerHandle,
        state: Arc<RwLock<NotifierState<E>>>,
    ) -> Self {
        Self {
            stream,
            handle,
            state,
        }
    }

    pub fn close(&mut self) {
        self.stream.close()
    }
}

impl<E> Stream for NotifierStream<E> {
    type Item = E;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.stream).poll_next(cx)
    }
}

impl<E> Drop for NotifierStream<E> {
    fn drop(&mut self) {
        self.state.write().deregister(self.handle);
    }
}

#[derive(Default)]
pub struct Notifier<E> {
    state: Arc<RwLock<NotifierState<E>>>,
}

impl<E> Notifier<E> {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(NotifierState::new())),
        }
    }

    pub fn register<T: Listener<E> + 'static>(&self, listener: T) -> ListenerHandle {
        self.state.write().register(listener)
    }

    pub fn deregister(&self, handle: ListenerHandle) {
        self.state.write().deregister(handle);
    }

    pub fn notify(&self, event: E) {
        self.state.read().notify(event);
    }
}

impl<E: Clone + Send + 'static> Notifier<E> {
    pub fn as_stream(&mut self) -> NotifierStream<E> {
        let (tx, rx) = mpsc::unbounded_channel();
        let handle = self.register(move |event: &E| {
            if let Err(e) = tx.send(event.clone()) {
                log::error!("Failed to send event to channel: {}", e);
            }
        });

        let state = Arc::clone(&self.state);
        NotifierStream::new(UnboundedReceiverStream::new(rx), handle, state)
    }
}

pub fn weak_listener<T, E, C>(weak_ref: Weak<T>, closure: C) -> impl Listener<E>
where
    C: Fn(Arc<T>, &E) + Send + Sync,
    T: Send + Sync,
{
    move |event: &E| {
        if let Some(arc) = weak_ref.upgrade() {
            closure(arc, event);
        }
    }
}

pub fn weak_passthru_listener<T, E, C>(weak_ref: Weak<T>, closure: C) -> impl PassThroughListener<E>
where
    C: Fn(Arc<T>, E) + Send + Sync,
    T: Send + Sync,
{
    move |event: E| {
        if let Some(arc) = weak_ref.upgrade() {
            closure(arc, event);
        }
    }
}

pub trait PassThroughListener<E>: Send + Sync {
    fn on_event(&self, event: E);
}

impl<E, F: Fn(E)> PassThroughListener<E> for F
where
    F: Send + Sync,
{
    fn on_event(&self, event: E) {
        self(event);
    }
}

pub struct PassThroughNotifier<'l, E> {
    listener: Option<Box<dyn PassThroughListener<E> + 'l>>,
}

impl<E> Default for PassThroughNotifier<'_, E> {
    fn default() -> Self {
        Self { listener: None }
    }
}

impl<'l, E> PassThroughNotifier<'l, E> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<T: PassThroughListener<E> + 'l>(&mut self, listener: T) {
        self.listener = Some(Box::new(listener));
    }

    pub fn deregister(&mut self) {
        self.listener = None;
    }

    pub fn notify(&self, event: E) {
        if let Some(ref listener) = self.listener {
            listener.on_event(event);
        }
    }
}
