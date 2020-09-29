use super::*;

pub struct MemoryStore<H> {
    pub inner: Vec<H>,
}

impl<H: Clone> MemoryStore<H> {
    pub fn new() -> Self {
        MemoryStore { inner: vec![] }
    }

    pub fn transaction(&mut self) -> MemoryTransaction<H, Self> {
        MemoryTransaction::new(self)
    }
}

impl<H: Clone> Store<H> for MemoryStore<H> {
    fn push(&mut self, elem: H) {
        self.inner.push(elem);
    }

    fn append(&mut self, mut elems: Vec<H>) {
        self.inner.append(&mut elems)
    }

    fn remove_back(&mut self, num_elems: usize) {
        self.inner.truncate(self.inner.len() - num_elems);
    }

    fn get(&self, pos: usize) -> Option<H> {
        self.inner.get(pos).cloned()
    }

    fn len(&self) -> usize {
        self.inner.len()
    }
}

pub struct MemoryTransaction<'a, H, S> {
    store: &'a mut S,
    /// The position of the transaction's data within the store.
    tx_pos: usize,
    data: Vec<H>,
}

impl<'a, H: Clone, S: Store<H>> MemoryTransaction<'a, H, S> {
    pub fn new(store: &'a mut S) -> Self {
        MemoryTransaction {
            tx_pos: store.len(),
            store,
            data: vec![],
        }
    }

    pub fn commit(self) {
        if self.tx_pos < self.store.len() {
            self.store.remove_back(self.store.len() - self.tx_pos);
        }
        self.store.append(self.data);
    }

    pub fn abort(self) {}
}

impl<'a, H: Clone, S: Store<H>> Store<H> for MemoryTransaction<'a, H, S> {
    fn push(&mut self, elem: H) {
        self.data.push(elem);
    }

    fn append(&mut self, mut elems: Vec<H>) {
        self.data.append(&mut elems);
    }

    fn remove_back(&mut self, num_elems: usize) {
        if num_elems > self.data.len() {
            self.tx_pos -= num_elems - self.data.len();
            self.data.clear();
        } else {
            self.data.truncate(self.data.len() - num_elems);
        }
    }

    fn get(&self, pos: usize) -> Option<H> {
        if pos < self.tx_pos {
            self.store.get(pos)
        } else {
            self.data.get(pos - self.tx_pos).cloned()
        }
    }

    fn len(&self) -> usize {
        self.tx_pos + self.data.len()
    }
}
