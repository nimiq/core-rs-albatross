use std::ops;

use nimiq_database::{
    traits::{ReadTransaction, WriteTransaction},
    TableProxy, TransactionProxy as RawTransactionProxy,
    WriteTransactionProxy as RawWriteTransactionProxy,
};
use nimiq_primitives::{
    key_nibbles::KeyNibbles,
    trie::{
        trie_diff::{TrieDiffBuilder, ValueChange},
        trie_node::TrieNode,
    },
};

pub(crate) trait TransactionExt {
    fn get_node(&self, db: &TableProxy, key: &KeyNibbles) -> Option<TrieNode>;
}

impl<'env> TransactionExt for RawTransactionProxy<'env> {
    fn get_node(&self, db: &TableProxy, key: &KeyNibbles) -> Option<TrieNode> {
        let mut node: TrieNode = self.get(db, key)?;
        node.key = key.clone();
        Some(node)
    }
}

impl<'txn, 'env> TransactionExt for WriteTransactionProxy<'txn, 'env> {
    fn get_node(&self, db: &TableProxy, key: &KeyNibbles) -> Option<TrieNode> {
        self.raw.get_node(db, key)
    }
}

pub(crate) enum OldValue {
    Unchanged,
    None,
    Some(Vec<u8>),
}

impl From<Option<Vec<u8>>> for OldValue {
    fn from(old_value: Option<Vec<u8>>) -> OldValue {
        match old_value {
            None => OldValue::None,
            Some(v) => OldValue::Some(v),
        }
    }
}

pub struct WriteTransactionProxy<'txn, 'env> {
    raw: &'txn mut RawWriteTransactionProxy<'env>,
    diff: Option<TrieDiffBuilder>,
}

impl<'txn, 'env> From<&'txn mut RawWriteTransactionProxy<'env>>
    for WriteTransactionProxy<'txn, 'env>
{
    fn from(raw: &'txn mut RawWriteTransactionProxy<'env>) -> WriteTransactionProxy<'txn, 'env> {
        WriteTransactionProxy { raw, diff: None }
    }
}

impl<'txn, 'env> WriteTransactionProxy<'txn, 'env> {
    pub fn start_recording(&mut self) {
        assert!(self.diff.is_none(), "cannot stack change recordings");
        self.diff = Some(Default::default());
    }
    pub fn stop_recording(&mut self) -> TrieDiffBuilder {
        self.diff
            .take()
            .expect("cannot stop change recording while none is active")
    }
    fn record_value_change(
        &mut self,
        key: &KeyNibbles,
        old_value: OldValue,
        new_value: Option<&[u8]>,
    ) {
        if let Some(diff) = self.diff.as_mut() {
            let value_change = match (old_value, new_value) {
                (OldValue::Unchanged, _) => return,
                (OldValue::None, None) => return,
                (OldValue::None, Some(n)) => ValueChange::Insert(n.to_owned()),
                (OldValue::Some(o), None) => ValueChange::Delete(o),
                (OldValue::Some(o), Some(n)) => {
                    if o == n {
                        return;
                    }
                    ValueChange::Update(o, n.to_owned())
                }
            };
            diff.add_change(key.clone(), value_change);
        }
    }
    pub(crate) fn clear_database(&mut self, db: &TableProxy) {
        self.raw.clear_database(db)
    }
    pub(crate) fn put_node(&mut self, db: &TableProxy, node: &TrieNode, old_value: OldValue) {
        self.record_value_change(&node.key, old_value, node.value.as_deref());
        self.raw.put_reserve(db, &node.key, node);
    }
    pub(crate) fn remove_node(&mut self, db: &TableProxy, key: &KeyNibbles, old_value: OldValue) {
        self.record_value_change(key, old_value, None);
        self.raw.remove(db, key);
    }
    pub fn raw(&mut self) -> &mut RawWriteTransactionProxy<'env> {
        self.raw
    }
}

impl<'txn, 'env> ops::Deref for WriteTransactionProxy<'txn, 'env> {
    type Target = RawTransactionProxy<'env>;
    fn deref(&self) -> &RawTransactionProxy<'env> {
        self.raw
    }
}
