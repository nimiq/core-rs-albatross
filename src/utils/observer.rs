pub trait Listener<E: Copy> {
    fn on_event(&self, event: E);
}

impl<E: Copy, F: Fn(E)> Listener<E> for F {
    fn on_event(&self, event: E) {
        self(event);
    }
}

pub type ListenerHandle = usize;

pub struct Notifier<E> {
    listeners: Vec<(ListenerHandle, Box<Listener<E>>)>,
    next_handle: ListenerHandle
}

impl<E: Copy> Notifier<E> {
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
            listener.on_event(event);
        }
    }
}
