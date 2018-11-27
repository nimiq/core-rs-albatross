//! A trait describing a queue functionality.
//!
//! The `Queue` allows enqueue and dequeue operations.

use std::hash::Hash;

use crate::linked_list::LinkedList;
use crate::unique_linked_list::UniqueLinkedList;

/// A trait describing a queue functionality.
///
/// The `Queue` allows enqueue and dequeue operations.
pub trait Queue<T> {
    /// Returns `true` if the queue is empty.
    ///
    /// This operation should compute in O(1) time.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::{UniqueLinkedList, Queue};
    ///
    /// let mut dl = UniqueLinkedList::new();
    /// assert!(dl.is_empty());
    ///
    /// dl.enqueue("foo");
    /// assert!(!dl.is_empty());
    /// ```
    fn is_empty(&self) -> bool;

    /// Returns the length of the queue.
    ///
    /// This operation should compute in O(1) time.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::{UniqueLinkedList, Queue};
    ///
    /// let mut dl = UniqueLinkedList::new();
    ///
    /// dl.enqueue(2);
    /// assert_eq!(dl.len(), 1);
    ///
    /// dl.enqueue(1);
    /// assert_eq!(dl.len(), 2);
    ///
    /// dl.enqueue(3);
    /// assert_eq!(dl.len(), 3);
    /// ```
    fn len(&self) -> usize;

    /// Removes all elements from the queue.
    ///
    /// This operation should compute in O(n) time.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::{UniqueLinkedList, Queue};
    ///
    /// let mut dl = UniqueLinkedList::new();
    ///
    /// dl.enqueue(2);
    /// dl.enqueue(1);
    /// assert_eq!(dl.len(), 2);
    /// assert_eq!(dl.peek(), Some(&2));
    ///
    /// dl.clear();
    /// assert_eq!(dl.len(), 0);
    /// assert_eq!(dl.peek(), None);
    /// ```
    fn clear(&mut self);

    /// Provides a reference to the front element, or `None` if the queue is
    /// empty.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::{UniqueLinkedList, Queue};
    ///
    /// let mut dl = UniqueLinkedList::new();
    /// assert_eq!(dl.peek(), None);
    ///
    /// dl.enqueue(1);
    /// assert_eq!(dl.peek(), Some(&1));
    /// ```
    fn peek(&self) -> Option<&T>;

    /// Removes the first element and returns it, or `None` if the queue is
    /// empty.
    ///
    /// This operation should compute in amortized O(1) time.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::{UniqueLinkedList, Queue};
    ///
    /// let mut d = UniqueLinkedList::new();
    /// assert_eq!(d.dequeue(), None);
    ///
    /// d.enqueue(1);
    /// d.enqueue(3);
    /// assert_eq!(d.dequeue(), Some(1));
    /// assert_eq!(d.dequeue(), Some(3));
    /// assert_eq!(d.dequeue(), None);
    /// ```
    fn dequeue(&mut self) -> Option<T>;

    /// Removes a given number of elements from the front of the queue
    /// and returns them, or `None` if the queue is
    /// empty.
    ///
    /// This operation should compute in amortized O(`n`) time.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::{UniqueLinkedList, Queue};
    ///
    /// let mut d = UniqueLinkedList::new();
    /// assert_eq!(d.dequeue_multi(1), vec![]);
    ///
    /// d.enqueue(1);
    /// d.enqueue(3);
    /// assert_eq!(d.dequeue_multi(0), vec![]);
    /// assert_eq!(d.dequeue_multi(1), vec![1]);
    /// assert_eq!(d.dequeue_multi(2), vec![3]);
    /// ```
    fn dequeue_multi(&mut self, n: usize) -> Vec<T>;

    /// Appends an element to the back of a queue if it is not yet present in the queue.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::{UniqueLinkedList, Queue};
    ///
    /// let mut d = UniqueLinkedList::new();
    /// d.enqueue(1);
    /// d.enqueue(3);
    /// assert_eq!(1, *d.peek().unwrap());
    /// ```
    fn enqueue(&mut self, elt: T);

    /// Moves all elements from `other` to the end of the queue.
    ///
    /// This reuses all the nodes from `other` and moves them into `self`. After
    /// this operation, `other` becomes empty.
    ///
    /// This operation computes in O(n) time and O(1) memory.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::{UniqueLinkedList, Queue};
    ///
    /// let mut queue1 = UniqueLinkedList::new();
    /// queue1.enqueue('a');
    ///
    /// let mut queue2 = UniqueLinkedList::new();
    /// queue2.enqueue('b');
    /// queue2.enqueue('c');
    ///
    /// queue1.append(&mut queue2);
    ///
    /// let mut iter = queue1.iter();
    /// assert_eq!(iter.next(), Some(&'a'));
    /// assert_eq!(iter.next(), Some(&'b'));
    /// assert_eq!(iter.next(), Some(&'c'));
    /// assert!(iter.next().is_none());
    ///
    /// assert!(queue2.is_empty());
    /// ```
    fn append(&mut self, other: &mut Self);

    /// Appends an element to the back of a queue and removes equal elements.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::{UniqueLinkedList, Queue};
    ///
    /// let mut d = UniqueLinkedList::new();
    /// d.enqueue(1);
    /// d.enqueue(3);
    /// d.requeue(1);
    /// assert_eq!(d.dequeue(), Some(3));
    /// assert_eq!(*d.peek().unwrap(), 1);
    /// ```
    fn requeue(&mut self, elt: T);
}

impl<T> Queue<T> for UniqueLinkedList<T>
    where T: Hash + Eq {
    #[inline]
    fn is_empty(&self) -> bool {
        UniqueLinkedList::is_empty(self)
    }

    #[inline]
    fn len(&self) -> usize {
        UniqueLinkedList::len(self)
    }

    #[inline]
    fn clear(&mut self) {
        UniqueLinkedList::clear(self)
    }

    #[inline]
    fn peek(&self) -> Option<&T> {
        UniqueLinkedList::front(self)
    }

    #[inline]
    fn dequeue(&mut self) -> Option<T> {
        UniqueLinkedList::pop_front(self)
    }

    #[inline]
    fn dequeue_multi(&mut self, n: usize) -> Vec<T> {
        let mut v = Vec::new();
        for _ in 0..n {
            match self.dequeue() {
                Some(elt) => v.push(elt),
                None => return v,
            }
        }
        v
    }

    #[inline]
    fn enqueue(&mut self, elt: T) {
        UniqueLinkedList::push_back(self, elt);
    }

    #[inline]
    fn append(&mut self, other: &mut Self) {
        UniqueLinkedList::append(self, other);
    }

    #[inline]
    fn requeue(&mut self, elt: T) {
        UniqueLinkedList::remove(self, &elt);
        self.enqueue(elt);
    }
}

impl<T> Queue<T> for LinkedList<T> {
    #[inline]
    fn is_empty(&self) -> bool {
        LinkedList::is_empty(self)
    }

    #[inline]
    fn len(&self) -> usize {
        LinkedList::len(self)
    }

    #[inline]
    fn clear(&mut self) {
        LinkedList::clear(self)
    }

    #[inline]
    fn peek(&self) -> Option<&T> {
        LinkedList::front(self)
    }

    #[inline]
    fn dequeue(&mut self) -> Option<T> {
        LinkedList::pop_front(self)
    }

    #[inline]
    fn dequeue_multi(&mut self, n: usize) -> Vec<T> {
        let mut v = Vec::new();
        for _ in 0..n {
            match self.dequeue() {
                Some(elt) => v.push(elt),
                None => return v,
            }
        }
        v
    }

    #[inline]
    fn enqueue(&mut self, elt: T) {
        LinkedList::push_back(self, elt);
    }

    #[inline]
    fn append(&mut self, other: &mut Self) {
        LinkedList::append(self, other);
    }

    #[inline]
    fn requeue(&mut self, elt: T) {
        self.enqueue(elt);
    }
}