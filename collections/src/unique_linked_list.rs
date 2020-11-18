//! A doubly-linked list with owned nodes and a unique constraint enforced by a `HashMap`.
//!
//! As with the `HashMap` type, a `UniqueLinkedList` requires that the elements
//! implement the `Eq` and `Hash` traits. This can frequently be achieved by
//! using `#[derive(PartialEq, Eq, Hash)]`. If you implement these yourself,
//! it is important that the following property holds:
//!
//! ```text
//! k1 == k2 -> hash(k1) == hash(k2)
//! ```
//!
//! In other words, if two keys are equal, their hashes must be equal.
//!
//!
//! It is a logic error for an item to be modified in such a way that the
//! item's hash, as determined by the `Hash` trait, or its equality, as
//! determined by the `Eq` trait, changes while it is in the set. This is
//! normally only possible through `Cell`, `RefCell`, global state, I/O, or
//! unsafe code.
//!
//! The `UniqueLinkedList` allows pushing and popping elements at either end
//! in amortized constant time.

use std::borrow::Borrow;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::convert::AsRef;
use std::fmt;
use std::hash::Hash;
use std::hash::Hasher;
use std::iter::FromIterator;
use std::iter::FusedIterator;
use std::ptr::NonNull;
use std::rc::Rc;

use crate::linked_list::{IntoIter as LinkedListIntoIter, Iter as LinkedListIter, LinkedList, Node};

/// A doubly-linked list with owned nodes and a unique constraint enforced by a HashMap.
///
/// As with the `HashMap` type, a `UniqueLinkedList` requires that the elements
/// implement the `Eq` and `Hash` traits. This can frequently be achieved by
/// using `#[derive(PartialEq, Eq, Hash)]`. If you implement these yourself,
/// it is important that the following property holds:
///
/// ```text
/// k1 == k2 -> hash(k1) == hash(k2)
/// ```
///
/// In other words, if two keys are equal, their hashes must be equal.
///
///
/// It is a logic error for an item to be modified in such a way that the
/// item's hash, as determined by the `Hash` trait, or its equality, as
/// determined by the `Eq` trait, changes while it is in the set. This is
/// normally only possible through `Cell`, `RefCell`, global state, I/O, or
/// unsafe code.
///
/// The `UniqueLinkedList` allows pushing and popping elements at either end
/// in amortized constant time.
pub struct UniqueLinkedList<T> {
    pub(super) list: LinkedList<Rc<T>>,
    pub(super) map: HashMap<Rc<T>, NonNull<Node<Rc<T>>>>,
}

/// An iterator over the elements of a `UniqueLinkedList`.
///
/// This `struct` is created by the [`iter`] method on [`UniqueLinkedList`]. See its
/// documentation for more.
///
/// [`iter`]: struct.UniqueLinkedList.html#method.iter
/// [`UniqueLinkedList`]: struct.UniqueLinkedList.html
#[derive(Clone, Debug)]
pub struct Iter<'a, T: 'a> {
    pub(super) iter: LinkedListIter<'a, Rc<T>>,
}

/// An owning iterator over the elements of a `UniqueLinkedList`.
///
/// This `struct` is created by the [`into_iter`] method on [`UniqueLinkedList`][`UniqueLinkedList`]
/// (provided by the `IntoIterator` trait). See its documentation for more.
///
/// [`into_iter`]: struct.UniqueLinkedList.html#method.into_iter
/// [`UniqueLinkedList`]: struct.UniqueLinkedList.html
#[derive(Clone)]
pub struct IntoIter<T> {
    pub(super) iter: LinkedListIntoIter<Rc<T>>,
}

impl<T> Default for UniqueLinkedList<T>
where
    T: Hash + Eq,
{
    /// Creates an empty `UniqueLinkedList<T>`.
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T> UniqueLinkedList<T>
where
    T: Hash + Eq,
{
    /// Creates an empty `UniqueLinkedList`.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_collections::UniqueLinkedList;
    ///
    /// let list: UniqueLinkedList<u32> = UniqueLinkedList::new();
    /// ```
    #[inline]
    pub fn new() -> Self {
        UniqueLinkedList {
            list: LinkedList::new(),
            map: HashMap::new(),
        }
    }

    /// Moves all elements from `other` to the end of the list.
    ///
    /// This reuses all the nodes from `other` and moves them into `self`. After
    /// this operation, `other` becomes empty.
    ///
    /// This operation computes in O(n) time and O(1) memory.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_collections::UniqueLinkedList;
    ///
    /// let mut list1 = UniqueLinkedList::new();
    /// list1.push_back('a');
    ///
    /// let mut list2 = UniqueLinkedList::new();
    /// list2.push_back('b');
    /// list2.push_back('c');
    ///
    /// list1.append(&mut list2);
    ///
    /// let mut iter = list1.iter();
    /// assert_eq!(iter.next(), Some(&'a'));
    /// assert_eq!(iter.next(), Some(&'b'));
    /// assert_eq!(iter.next(), Some(&'c'));
    /// assert!(iter.next().is_none());
    ///
    /// assert!(list2.is_empty());
    /// ```
    pub fn append(&mut self, other: &mut Self) {
        while let Some(elt) = other.pop_front() {
            self.push_back(elt);
        }
    }

    /// Provides a forward iterator.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_collections::UniqueLinkedList;
    ///
    /// let mut list: UniqueLinkedList<u32> = UniqueLinkedList::new();
    ///
    /// list.push_back(0);
    /// list.push_back(1);
    /// list.push_back(2);
    ///
    /// let mut iter = list.iter();
    /// assert_eq!(iter.next(), Some(&0));
    /// assert_eq!(iter.next(), Some(&1));
    /// assert_eq!(iter.next(), Some(&2));
    /// assert_eq!(iter.next(), None);
    /// ```
    #[inline]
    pub fn iter(&self) -> Iter<T> {
        Iter { iter: self.list.iter() }
    }

    /// Returns `true` if the `UniqueLinkedList` is empty.
    ///
    /// This operation should compute in O(1) time.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_collections::UniqueLinkedList;
    ///
    /// let mut dl = UniqueLinkedList::new();
    /// assert!(dl.is_empty());
    ///
    /// dl.push_front("foo");
    /// assert!(!dl.is_empty());
    /// ```
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Returns the length of the `UniqueLinkedList`.
    ///
    /// This operation should compute in O(1) time.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_collections::UniqueLinkedList;
    ///
    /// let mut dl = UniqueLinkedList::new();
    ///
    /// dl.push_front(2);
    /// assert_eq!(dl.len(), 1);
    ///
    /// dl.push_front(1);
    /// assert_eq!(dl.len(), 2);
    ///
    /// dl.push_back(3);
    /// assert_eq!(dl.len(), 3);
    /// ```
    #[inline]
    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// Removes all elements from the `UniqueLinkedList`.
    ///
    /// This operation should compute in O(n) time.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_collections::UniqueLinkedList;
    ///
    /// let mut dl = UniqueLinkedList::new();
    ///
    /// dl.push_front(2);
    /// dl.push_front(1);
    /// assert_eq!(dl.len(), 2);
    /// assert_eq!(dl.front(), Some(&1));
    ///
    /// dl.clear();
    /// assert_eq!(dl.len(), 0);
    /// assert_eq!(dl.front(), None);
    /// ```
    #[inline]
    pub fn clear(&mut self) {
        self.list.clear();
        self.map.clear();
    }

    /// Returns `true` if the `UniqueLinkedList` contains an element equal to the
    /// given value.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_collections::UniqueLinkedList;
    ///
    /// let mut list: UniqueLinkedList<u32> = UniqueLinkedList::new();
    ///
    /// list.push_back(0);
    /// list.push_back(1);
    /// list.push_back(2);
    ///
    /// assert_eq!(list.contains(&0), true);
    /// assert_eq!(list.contains(&10), false);
    /// ```
    pub fn contains<Q: ?Sized>(&self, x: &Q) -> bool
    where
        Rc<T>: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.map.contains_key(x)
    }

    /// Removes and returns the element equal to the
    /// given value if present, otherwise returns `None`.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_collections::UniqueLinkedList;
    ///
    /// let mut list: UniqueLinkedList<u32> = UniqueLinkedList::new();
    ///
    /// list.push_back(0);
    ///
    /// assert_eq!(list.remove(&0), Some(0));
    /// assert_eq!(list.remove(&10), None);
    /// ```
    pub fn remove<Q: ?Sized>(&mut self, x: &Q) -> Option<T>
    where
        Rc<T>: Borrow<Q>,
        Q: Hash + Eq,
    {
        // Remove and drop node.
        let elt = unsafe {
            let (elt, node) = self.map.remove_entry(x)?;
            self.list.unlink_node(node);
            Box::from_raw(node.as_ptr()); // This drops the node's Box.
            elt
        };
        let elt = Rc::try_unwrap(elt).ok().expect("Internal contract requires a single strong reference");
        Some(elt)
    }

    /// Provides a reference to the front element, or `None` if the list is
    /// empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_collections::UniqueLinkedList;
    ///
    /// let mut dl = UniqueLinkedList::new();
    /// assert_eq!(dl.front(), None);
    ///
    /// dl.push_front(1);
    /// assert_eq!(dl.front(), Some(&1));
    /// ```
    #[inline]
    pub fn front(&self) -> Option<&T> {
        self.list.front().map(AsRef::as_ref)
    }

    /// Provides a reference to the back element, or `None` if the list is
    /// empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_collections::UniqueLinkedList;
    ///
    /// let mut dl = UniqueLinkedList::new();
    /// assert_eq!(dl.back(), None);
    ///
    /// dl.push_back(1);
    /// assert_eq!(dl.back(), Some(&1));
    /// ```
    #[inline]
    pub fn back(&self) -> Option<&T> {
        self.list.back().map(AsRef::as_ref)
    }

    /// Adds an element first in the list if it is not yet present in the list
    ///
    /// This operation should compute in amortized O(1) time.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_collections::UniqueLinkedList;
    ///
    /// let mut dl = UniqueLinkedList::new();
    ///
    /// dl.push_front(2);
    /// assert_eq!(dl.front().unwrap(), &2);
    ///
    /// dl.push_front(1);
    /// assert_eq!(dl.front().unwrap(), &1);
    /// ```
    pub fn push_front(&mut self, elt: T) {
        if self.contains(&elt) {
            return;
        }

        let elt = Rc::new(elt);
        self.list.push_front(elt.clone());
        let ptr = self.list.head.unwrap();
        self.map.insert(elt, ptr);
    }

    /// Removes the first element and returns it, or `None` if the list is
    /// empty.
    ///
    /// This operation should compute in amortized O(1) time.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_collections::UniqueLinkedList;
    ///
    /// let mut d = UniqueLinkedList::new();
    /// assert_eq!(d.pop_front(), None);
    ///
    /// d.push_front(1);
    /// d.push_front(3);
    /// assert_eq!(d.pop_front(), Some(3));
    /// assert_eq!(d.pop_front(), Some(1));
    /// assert_eq!(d.pop_front(), None);
    /// ```
    pub fn pop_front(&mut self) -> Option<T> {
        let elt = self.list.pop_front()?;
        self.map.remove(&elt).expect("Internal contract requires value to be present");
        let elt = Rc::try_unwrap(elt).ok().expect("Internal contract requires a single strong reference");
        Some(elt)
    }

    /// Appends an element to the back of a list if it is not yet present in the list
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_collections::UniqueLinkedList;
    ///
    /// let mut d = UniqueLinkedList::new();
    /// d.push_back(1);
    /// d.push_back(3);
    /// assert_eq!(3, *d.back().unwrap());
    /// ```
    pub fn push_back(&mut self, elt: T) {
        if self.contains(&elt) {
            return;
        }

        let elt = Rc::new(elt);
        self.list.push_back(elt.clone());
        let ptr = self.list.tail.unwrap();
        self.map.insert(elt, ptr);
    }

    /// Removes the last element from a list and returns it, or `None` if
    /// it is empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use nimiq_collections::UniqueLinkedList;
    ///
    /// let mut d = UniqueLinkedList::new();
    /// assert_eq!(d.pop_back(), None);
    /// d.push_back(1);
    /// d.push_back(3);
    /// assert_eq!(d.pop_back(), Some(3));
    /// ```
    pub fn pop_back(&mut self) -> Option<T> {
        let elt = self.list.pop_back()?;
        self.map.remove(&elt).expect("Internal contract requires value to be present");
        let elt = Rc::try_unwrap(elt).ok().expect("Internal contract requires a single strong reference");
        Some(elt)
    }
}

impl<'a, T> Iterator for Iter<'a, T>
where
    T: Hash + Eq,
{
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<&'a T> {
        self.iter.next().map(AsRef::as_ref)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<'a, T> DoubleEndedIterator for Iter<'a, T>
where
    T: Hash + Eq,
{
    #[inline]
    fn next_back(&mut self) -> Option<&'a T> {
        self.iter.next_back().map(AsRef::as_ref)
    }
}

impl<'a, T> ExactSizeIterator for Iter<'a, T> where T: Hash + Eq {}

impl<'a, T> FusedIterator for Iter<'a, T> where T: Hash + Eq {}

impl<T> Iterator for IntoIter<T>
where
    T: Hash + Eq,
{
    type Item = T;

    #[inline]
    fn next(&mut self) -> Option<T> {
        self.iter
            .next()
            .map(|elt| Rc::try_unwrap(elt).ok().expect("Internal contract requires a single strong reference"))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<T> DoubleEndedIterator for IntoIter<T>
where
    T: Hash + Eq,
{
    #[inline]
    fn next_back(&mut self) -> Option<T> {
        self.iter
            .next_back()
            .map(|elt| Rc::try_unwrap(elt).ok().expect("Internal contract requires a single strong reference"))
    }
}

impl<T> ExactSizeIterator for IntoIter<T> where T: Hash + Eq {}

impl<T> FusedIterator for IntoIter<T> where T: Hash + Eq {}

impl<T> FromIterator<T> for UniqueLinkedList<T>
where
    T: Hash + Eq,
{
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut list = Self::new();
        list.extend(iter);
        list
    }
}

impl<T> IntoIterator for UniqueLinkedList<T>
where
    T: Hash + Eq,
{
    type Item = T;
    type IntoIter = IntoIter<T>;

    /// Consumes the list into an iterator yielding elements by value.
    #[inline]
    fn into_iter(mut self) -> IntoIter<T> {
        self.map.clear();
        IntoIter { iter: self.list.into_iter() }
    }
}

impl<'a, T> IntoIterator for &'a UniqueLinkedList<T>
where
    T: Hash + Eq,
{
    type Item = &'a T;
    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Iter<'a, T> {
        self.iter()
    }
}

impl<T> Extend<T> for UniqueLinkedList<T>
where
    T: Hash + Eq,
{
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for elt in iter {
            self.push_back(elt);
        }
    }
}

impl<'a, T: 'a + Copy> Extend<&'a T> for UniqueLinkedList<T>
where
    T: Hash + Eq,
{
    fn extend<I: IntoIterator<Item = &'a T>>(&mut self, iter: I) {
        self.extend(iter.into_iter().cloned());
    }
}

impl<T: PartialEq> PartialEq for UniqueLinkedList<T>
where
    T: Hash + Eq,
{
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other)
    }
}

impl<T: Eq> Eq for UniqueLinkedList<T> where T: Hash + Eq {}

impl<T: PartialOrd> PartialOrd for UniqueLinkedList<T>
where
    T: Hash + Eq,
{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.iter().partial_cmp(other)
    }
}

impl<T: Ord> Ord for UniqueLinkedList<T>
where
    T: Hash + Eq,
{
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.iter().cmp(other)
    }
}

impl<T: Clone> Clone for UniqueLinkedList<T>
where
    T: Hash + Eq,
{
    fn clone(&self) -> Self {
        self.iter().cloned().collect()
    }
}

impl<T: fmt::Debug> fmt::Debug for UniqueLinkedList<T>
where
    T: Hash + Eq,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_list().entries(self).finish()
    }
}

impl<T: Hash> Hash for UniqueLinkedList<T>
where
    T: Hash + Eq,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.len().hash(state);
        for elt in self {
            elt.hash(state);
        }
    }
}

// Ensure that `LinkedList` and its read-only iterators are covariant in their type parameters.
#[allow(dead_code)]
fn assert_covariance() {
    fn a<'a>(x: UniqueLinkedList<&'static str>) -> UniqueLinkedList<&'a str> {
        x
    }
    fn b<'i, 'a>(x: Iter<'i, &'static str>) -> Iter<'i, &'a str> {
        x
    }
    fn c<'a>(x: IntoIter<&'static str>) -> IntoIter<&'a str> {
        x
    }
}

unsafe impl<T: Send> Send for UniqueLinkedList<T> {}

unsafe impl<T: Sync> Sync for UniqueLinkedList<T> {}

unsafe impl<'a, T: Sync> Send for Iter<'a, T> {}

unsafe impl<'a, T: Sync> Sync for Iter<'a, T> {}

#[cfg(test)]
mod tests {
    use std::hash::Hash;
    use std::rc::Rc;
    use std::thread;
    use std::vec::Vec;

    use super::{Node, UniqueLinkedList};

    #[cfg(test)]
    fn list_from<T: Clone + Hash + Eq>(v: &[T]) -> UniqueLinkedList<T> {
        v.iter().cloned().collect()
    }

    pub fn check_links<T>(list: &UniqueLinkedList<T>) {
        let list = &list.list;
        unsafe {
            let mut len = 0;
            let mut last_ptr: Option<&Node<Rc<T>>> = None;
            let mut node_ptr: &Node<Rc<T>>;
            match list.head {
                None => {
                    // tail node should also be None.
                    assert!(list.tail.is_none());
                    assert_eq!(0, list.len);
                    return;
                }
                Some(node) => node_ptr = &*node.as_ptr(),
            }
            loop {
                match (last_ptr, node_ptr.prev) {
                    (None, None) => {}
                    (None, _) => panic!("prev link for head"),
                    (Some(p), Some(pptr)) => {
                        assert_eq!(p as *const Node<Rc<T>>, pptr.as_ptr() as *const Node<Rc<T>>);
                    }
                    _ => panic!("prev link is none, not good"),
                }
                match node_ptr.next {
                    Some(next) => {
                        last_ptr = Some(node_ptr);
                        node_ptr = &*next.as_ptr();
                        len += 1;
                    }
                    None => {
                        len += 1;
                        break;
                    }
                }
            }

            // verify that the tail node points to the last node.
            let tail = list.tail.as_ref().expect("some tail node").as_ref();
            assert_eq!(tail as *const Node<Rc<T>>, node_ptr as *const Node<Rc<T>>);
            // check that len matches interior links.
            assert_eq!(len, list.len);
        }
    }

    #[test]
    fn test_append() {
        // Empty to empty
        {
            let mut m = UniqueLinkedList::<i32>::new();
            let mut n = UniqueLinkedList::new();
            m.append(&mut n);
            check_links(&m);
            assert_eq!(m.len(), 0);
            assert_eq!(n.len(), 0);
        }
        // Non-empty to empty
        {
            let mut m = UniqueLinkedList::new();
            let mut n = UniqueLinkedList::new();
            n.push_back(2);
            m.append(&mut n);
            check_links(&m);
            assert_eq!(m.len(), 1);
            assert_eq!(m.pop_back(), Some(2));
            assert_eq!(n.len(), 0);
            check_links(&m);
        }
        // Empty to non-empty
        {
            let mut m = UniqueLinkedList::new();
            let mut n = UniqueLinkedList::new();
            m.push_back(2);
            m.append(&mut n);
            check_links(&m);
            assert_eq!(m.len(), 1);
            assert_eq!(m.pop_back(), Some(2));
            check_links(&m);
        }

        // Non-empty to non-empty
        let v = vec![1, 2, 3, 4, 5];
        let u = vec![9, 8, 1, 2, 3, 4, 5];
        let mut m = list_from(&v);
        let mut n = list_from(&u);
        m.append(&mut n);
        check_links(&m);
        let sum = vec![1, 2, 3, 4, 5, 9, 8];
        assert_eq!(sum.len(), m.len());
        for elt in sum {
            assert_eq!(m.pop_front(), Some(elt))
        }
        assert_eq!(n.len(), 0);
        // let's make sure it's working properly, since we
        // did some direct changes to private members
        n.push_back(7);
        assert_eq!(n.len(), 1);
        assert_eq!(n.pop_front(), Some(7));
        check_links(&n);
    }

    #[test]
    #[cfg_attr(target_os = "emscripten", ignore)]
    fn test_send() {
        let n = list_from(&[1, 2, 3]);
        thread::spawn(move || {
            check_links(&n);
            let a: &[_] = &[&1, &2, &3];
            assert_eq!(a, &*n.iter().collect::<Vec<_>>());
        })
        .join()
        .ok()
        .unwrap();
    }

    #[test]
    fn it_can_correctly_pop_elements() {
        let mut q = UniqueLinkedList::new();

        assert_eq!(q.len(), 0);

        q.push_back(3);
        q.push_back(1);
        q.push_back(2);

        assert_eq!(q.len(), 3);
        assert_eq!(q.pop_front(), Some(3));
        assert_eq!(q.len(), 2);
        assert_eq!(q.pop_front(), Some(1));
        assert_eq!(q.len(), 1);
        assert_eq!(q.pop_front(), Some(2));
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn it_can_clear_itself() {
        let mut q = UniqueLinkedList::new();

        assert_eq!(q.len(), 0);

        q.push_front(3);
        q.push_front(1);
        q.push_front(2);

        assert_eq!(q.len(), 3);
        assert_eq!(q.is_empty(), false);
        q.clear();
        assert_eq!(q.len(), 0);
        assert_eq!(q.pop_front(), None);
        assert_eq!(q.pop_back(), None);
        assert_eq!(q.is_empty(), true);
    }

    #[test]
    fn it_can_peek() {
        let mut q = UniqueLinkedList::new();

        q.push_back(1);
        q.push_front(3);

        assert_eq!(q.front(), Some(&3));
        assert_eq!(q.back(), Some(&1));
        q.pop_front();
        assert_eq!(q.front(), Some(&1));
        assert_eq!(q.back(), Some(&1));
        q.pop_back();
        assert_eq!(q.front(), None);
        assert_eq!(q.back(), None);
    }

    #[test]
    fn it_can_push_unique() {
        let mut q = UniqueLinkedList::new();

        q.push_front(3);
        q.push_back(1);
        q.push_front(2);
        q.push_back(3);
        q.push_front(2);

        assert_eq!(q.len(), 3);
        assert_eq!(q.pop_front(), Some(2));
        assert_eq!(q.pop_front(), Some(3));
        assert_eq!(q.pop_front(), Some(1));
    }

    #[test]
    fn it_can_remove() {
        let mut q = UniqueLinkedList::new();
        q.extend(vec![3, 1, 2, 4, 5]);
        assert_eq!(q.remove(&2), Some(2));
        assert_eq!(q.remove(&3), Some(3));
        assert_eq!(q.remove(&5), Some(5));
        assert_eq!(q.remove(&9), None);
        assert_eq!(q.len(), 2);

        assert_eq!(q.pop_front(), Some(1));
        assert_eq!(q.pop_front(), Some(4));
    }

    #[test]
    fn it_can_test_contains() {
        let mut q = UniqueLinkedList::new();
        q.extend(vec![3, 1, 2, 4, 5]);

        assert_eq!(q.contains(&1), true);
        assert_eq!(q.contains(&8), false);
    }
}
