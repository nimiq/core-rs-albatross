use std::collections::{hash_map, HashMap};
use std::fmt;
use std::hash::Hash;
use tokio::sync::broadcast;

#[derive(Clone)]
pub enum Event<K, V> {
    Add(K, V),
    Remove(K),
}

pub struct ObservableHashMap<K: Clone + Eq + Hash, V: Clone> {
    inner: HashMap<K, V>,
    tx: broadcast::Sender<Event<K, V>>,
}

impl<K: Clone + Eq + Hash + fmt::Debug, V: Clone + fmt::Debug> fmt::Debug
    for ObservableHashMap<K, V>
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl<K: Clone + Eq + Hash, V: Clone> Default for ObservableHashMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Clone + Eq + Hash, V: Clone> From<HashMap<K, V>> for ObservableHashMap<K, V> {
    fn from(inner: HashMap<K, V>) -> ObservableHashMap<K, V> {
        let (tx, _) = broadcast::channel(64);
        ObservableHashMap { inner, tx }
    }
}

impl<K: Clone + Eq + Hash, V: Clone> ObservableHashMap<K, V> {
    pub fn new() -> Self {
        Self::from(HashMap::new())
    }
    pub fn insert(&mut self, k: K, v: V) -> Option<V> {
        let result = self.inner.insert(k.clone(), v.clone());
        if result.is_none() {
            // We don't care if there's no receiver active.
            let _ = self.tx.send(Event::Add(k, v));
        }
        result
    }
    pub fn remove(&mut self, k: &K) -> Option<V> {
        let result = self.inner.remove(k);
        if result.is_some() {
            // We don't care if there's no receiver active.
            let _ = self.tx.send(Event::Remove(k.clone()));
        }
        result
    }
    pub fn contains_key(&self, k: &K) -> bool {
        self.inner.contains_key(k)
    }
    pub fn get<'a>(&'a self, k: &K) -> Option<&'a V> {
        self.inner.get(k)
    }
    pub fn keys(&self) -> hash_map::Keys<K, V> {
        self.inner.keys()
    }
    pub fn subscribe(&self) -> broadcast::Receiver<Event<K, V>> {
        self.tx.subscribe()
    }
}
