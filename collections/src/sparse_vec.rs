use std::collections::LinkedList;
use std::convert::TryFrom;

/// This is a special vector implementation that has a O(1) remove function.
/// It never shrinks in size, but reuses available spaces as much as possible.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SparseVec<T, K = usize> {
    inner: Vec<Option<T>>,
    free_indices: LinkedList<K>,
}

impl<T, K> SparseVec<T, K>
where
    K: Into<usize> + TryFrom<usize> + Copy,
{
    pub fn new() -> Self {
        SparseVec {
            inner: Vec::new(),
            free_indices: LinkedList::new(),
        }
    }

    pub fn get(&self, index: K) -> Option<&T> {
        self.inner.get(index.into())?.as_ref()
    }

    pub fn get_mut(&mut self, index: K) -> Option<&mut T> {
        self.inner.get_mut(index.into())?.as_mut()
    }

    pub fn remove(&mut self, index: K) -> Option<T> {
        let value = self.inner.get_mut(index.into())?.take();
        if value.is_some() {
            self.free_indices.push_back(index);
        }
        value
    }

    /// Fails if vector reached its limit.
    pub fn insert(&mut self, value: T) -> Option<K> {
        if let Some(index) = self.free_indices.pop_front() {
            self.inner[index.into()].get_or_insert(value);
            Some(index)
        } else {
            let index = self.inner.len();
            self.inner.push(Some(value));
            K::try_from(index).ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_vec_can_store_objects() {
        let mut v = SparseVec::new();

        // Insert.
        let i1 = v.insert(5).unwrap();
        assert_eq!(i1, 0);
        let i2 = v.insert(5).unwrap();
        assert_eq!(i2, 1);

        // Read/Write access.
        assert_eq!(v.get(i1), Some(&5));
        *v.get_mut(i2).unwrap() = 8;
        assert_eq!(v.get(i2), Some(&8));
        assert_eq!(v.get(2_usize), None);
        assert_eq!(v.free_indices.len(), 0);

        // Remove.
        assert_eq!(v.remove(i1), Some(5));
        assert_eq!(v.get(i1), None);
        let i3 = v.insert(1).unwrap();
        assert_eq!(i3, 0);

        assert_eq!(v.remove(i2), Some(8));
        assert_eq!(v.remove(i2), None);
        assert_eq!(v.free_indices.len(), 1);

        let i4 = v.insert(2).unwrap();
        assert_eq!(i4, 1);
        assert_eq!(v.free_indices.len(), 0);

        let i5 = v.insert(4).unwrap();
        assert_eq!(i5, 2);
    }
}
