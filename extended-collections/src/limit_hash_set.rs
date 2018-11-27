use std::borrow::Borrow;
use std::fmt;
use std::hash::Hash;
use std::iter::Chain;
use std::iter::FromIterator;
use std::iter::FusedIterator;
use std::ops::BitAnd;
use std::ops::BitOr;
use std::ops::BitXor;
use std::ops::Sub;
use std::rc::Rc;

use crate::unique_linked_list::{IntoIter, Iter, UniqueLinkedList};

/// A hash set implemented as `UniqueLinkedList` that has a limit on the number of elements.
///
/// As with the `UniqueLinkedList` type, a `LimitHashSet` requires that the elements
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
/// # Examples
///
/// ```
/// use extended_collections::LimitHashSet;
/// // Type inference lets us omit an explicit type signature (which
/// // would be `LimitHashSet<String>` in this example).
/// let mut books = LimitHashSet::new(3);
///
/// // Add some books.
/// books.insert("A Dance With Dragons".to_string());
/// books.insert("To Kill a Mockingbird".to_string());
/// books.insert("The Odyssey".to_string());
/// books.insert("The Great Gatsby".to_string());
///
/// // Check for a specific one.
/// if !books.contains(&"The Winds of Winter".to_string()) {
///     println!("We have {} books, but The Winds of Winter ain't one.",
///              books.len());
/// }
///
/// // Remove a book.
/// books.remove(&"The Odyssey".to_string());
///
/// // Iterate over everything.
/// for book in &books {
///     println!("{}", book);
/// }
/// ```
///
/// The easiest way to use `LimitHashSet` with a custom type is to derive
/// `Eq` and `Hash`. We must also derive `PartialEq`, this will in the
/// future be implied by `Eq`.
///
/// ```
/// use extended_collections::LimitHashSet;
/// #[derive(Hash, Eq, PartialEq, Debug)]
/// struct Viking {
///     name: String,
///     power: usize,
/// }
///
/// let mut vikings = LimitHashSet::new(3);
///
/// vikings.insert(Viking { name: "Einar".to_string(), power: 9 });
/// vikings.insert(Viking { name: "Einar".to_string(), power: 9 });
/// vikings.insert(Viking { name: "Olaf".to_string(), power: 4 });
/// vikings.insert(Viking { name: "Harald".to_string(), power: 8 });
///
/// // Use derived implementation to print the vikings.
/// for x in &vikings {
///     println!("{:?}", x);
/// }
/// ```
#[derive(Clone)]
pub struct LimitHashSet<T>
    where T: Hash + Eq {
    list: UniqueLinkedList<T>,
    limit: usize,
}

impl<T> LimitHashSet<T>
    where T: Hash + Eq {
    /// Creates an empty `LimitHashSet` with a limit of `limit` elements.
    /// `limit` must be greater than zero or this function will panic.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    /// let set: LimitHashSet<i32> = LimitHashSet::new(10);
    /// ```
    #[inline]
    pub fn new(limit: usize) -> Self {
        if limit == 0 {
            panic!("Limit must be > 0");
        }
        LimitHashSet {
            list: UniqueLinkedList::new(),
            limit,
        }
    }

    /// An iterator visiting all elements in insertion order.
    /// The iterator element type is `&'a T`.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    /// let mut set = LimitHashSet::new(10);
    /// set.insert("a");
    /// set.insert("b");
    ///
    /// // Will print in an arbitrary order.
    /// for x in set.iter() {
    ///     println!("{}", x);
    /// }
    /// ```
    pub fn iter(&self) -> Iter<T> {
        self.list.iter()
    }

    /// Returns the number of elements in the set.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    ///
    /// let mut v = LimitHashSet::new(10);
    /// assert_eq!(v.len(), 0);
    /// v.insert(1);
    /// assert_eq!(v.len(), 1);
    /// ```
    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// Returns true if the set contains no elements.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    ///
    /// let mut v = LimitHashSet::new(10);
    /// assert!(v.is_empty());
    /// v.insert(1);
    /// assert!(!v.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Clears the set, removing all values.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    ///
    /// let mut v = LimitHashSet::new(10);
    /// v.insert(1);
    /// v.clear();
    /// assert!(v.is_empty());
    /// ```
    pub fn clear(&mut self) {
        self.list.clear()
    }

    /// Visits the values representing the difference,
    /// i.e. the values that are in `self` but not in `other`.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    /// let a: LimitHashSet<_> = [1, 2, 3].iter().cloned().collect();
    /// let b: LimitHashSet<_> = [4, 2, 3, 4].iter().cloned().collect();
    ///
    /// // Can be seen as `a - b`.
    /// for x in a.difference(&b) {
    ///     println!("{}", x); // Print 1
    /// }
    ///
    /// let diff: LimitHashSet<_> = a.difference(&b).collect();
    /// assert_eq!(diff, [1].iter().collect());
    ///
    /// // Note that difference is not symmetric,
    /// // and `b - a` means something else:
    /// let diff: LimitHashSet<_> = b.difference(&a).collect();
    /// assert_eq!(diff, [4].iter().collect());
    /// ```
    pub fn difference<'a>(&'a self, other: &'a LimitHashSet<T>) -> Difference<'a, T> {
        Difference {
            iter: self.iter(),
            other,
        }
    }

    /// Visits the values representing the symmetric difference,
    /// i.e. the values that are in `self` or in `other` but not in both.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    /// let a: LimitHashSet<_> = [1, 2, 3].iter().cloned().collect();
    /// let b: LimitHashSet<_> = [4, 2, 3, 4].iter().cloned().collect();
    ///
    /// // Print 1, 4 in arbitrary order.
    /// for x in a.symmetric_difference(&b) {
    ///     println!("{}", x);
    /// }
    ///
    /// let diff1: LimitHashSet<_> = a.symmetric_difference(&b).collect();
    /// let diff2: LimitHashSet<_> = b.symmetric_difference(&a).collect();
    ///
    /// assert_eq!(diff1, diff2);
    /// assert_eq!(diff1, [1, 4].iter().collect());
    /// ```
    pub fn symmetric_difference<'a>(&'a self,
                                    other: &'a LimitHashSet<T>)
                                    -> SymmetricDifference<'a, T> {
        SymmetricDifference { iter: self.difference(other).chain(other.difference(self)) }
    }

    /// Visits the values representing the intersection,
    /// i.e. the values that are both in `self` and `other`.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    /// let a: LimitHashSet<_> = [1, 2, 3].iter().cloned().collect();
    /// let b: LimitHashSet<_> = [4, 2, 3, 4].iter().cloned().collect();
    ///
    /// // Print 2, 3 in arbitrary order.
    /// for x in a.intersection(&b) {
    ///     println!("{}", x);
    /// }
    ///
    /// let intersection: LimitHashSet<_> = a.intersection(&b).collect();
    /// assert_eq!(intersection, [2, 3].iter().collect());
    /// ```
    pub fn intersection<'a>(&'a self, other: &'a LimitHashSet<T>) -> Intersection<'a, T> {
        Intersection {
            iter: self.iter(),
            other,
        }
    }

    /// Visits the values representing the union,
    /// i.e. all the values in `self` or `other`, without duplicates.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    /// let a: LimitHashSet<_> = [1, 2, 3].iter().cloned().collect();
    /// let b: LimitHashSet<_> = [4, 2, 3, 4].iter().cloned().collect();
    ///
    /// // Print 1, 2, 3, 4 in arbitrary order.
    /// for x in a.union(&b) {
    ///     println!("{}", x);
    /// }
    ///
    /// let union: LimitHashSet<_> = a.union(&b).collect();
    /// assert_eq!(union, [1, 2, 3, 4].iter().collect());
    /// ```
    pub fn union<'a>(&'a self, other: &'a LimitHashSet<T>) -> Union<'a, T> {
        Union { iter: self.iter().chain(other.difference(self)) }
    }

    /// Returns `true` if the set contains a value.
    ///
    /// The value may be any borrowed form of the set's value type, but
    /// `Hash` and `Eq` on the borrowed form *must* match those for
    /// the value type.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    ///
    /// let set: LimitHashSet<_> = [1, 2, 3].iter().cloned().collect();
    /// assert_eq!(set.contains(&1), true);
    /// assert_eq!(set.contains(&4), false);
    /// ```
    pub fn contains<Q: ?Sized>(&self, value: &Q) -> bool
        where Rc<T>: Borrow<Q>,
              Q: Hash + Eq
    {
        self.list.contains(value)
    }

    /// Returns a reference to the value in the set, if any, that is equal to the given value.
    ///
    /// The value may be any borrowed form of the set's value type, but
    /// [`Hash`] and [`Eq`] on the borrowed form *must* match those for
    /// the value type.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    ///
    /// let set: LimitHashSet<_> = [1, 2, 3].iter().cloned().collect();
    /// assert_eq!(set.get(&2), Some(&2));
    /// assert_eq!(set.get(&4), None);
    /// ```
    ///
    /// [`Eq`]: ../../std/cmp/trait.Eq.html
    /// [`Hash`]: ../../std/hash/trait.Hash.html
    pub fn get<Q: ?Sized>(&self, value: &Q) -> Option<&T>
        where Rc<T>: Borrow<Q>,
              Q: Hash + Eq
    {
        let (k, _) = self.list.map.get_key_value(value)?;
        Some(k.as_ref())
    }

    /// Returns `true` if `self` has no elements in common with `other`.
    /// This is equivalent to checking for an empty intersection.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    ///
    /// let a: LimitHashSet<_> = [1, 2, 3].iter().cloned().collect();
    /// let mut b = LimitHashSet::new(10);
    ///
    /// assert_eq!(a.is_disjoint(&b), true);
    /// b.insert(4);
    /// assert_eq!(a.is_disjoint(&b), true);
    /// b.insert(1);
    /// assert_eq!(a.is_disjoint(&b), false);
    /// ```
    pub fn is_disjoint(&self, other: &LimitHashSet<T>) -> bool {
        self.iter().all(|v| !other.contains(v))
    }

    /// Returns `true` if the set is a subset of another,
    /// i.e. `other` contains at least all the values in `self`.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    ///
    /// let sup: LimitHashSet<_> = [1, 2, 3].iter().cloned().collect();
    /// let mut set = LimitHashSet::new(10);
    ///
    /// assert_eq!(set.is_subset(&sup), true);
    /// set.insert(2);
    /// assert_eq!(set.is_subset(&sup), true);
    /// set.insert(4);
    /// assert_eq!(set.is_subset(&sup), false);
    /// ```
    pub fn is_subset(&self, other: &LimitHashSet<T>) -> bool {
        self.iter().all(|v| other.contains(v))
    }

    /// Returns `true` if the set is a superset of another,
    /// i.e. `self` contains at least all the values in `other`.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    ///
    /// let sub: LimitHashSet<_> = [1, 2].iter().cloned().collect();
    /// let mut set = LimitHashSet::new(10);
    ///
    /// assert_eq!(set.is_superset(&sub), false);
    ///
    /// set.insert(0);
    /// set.insert(1);
    /// assert_eq!(set.is_superset(&sub), false);
    ///
    /// set.insert(2);
    /// assert_eq!(set.is_superset(&sub), true);
    /// ```
    #[inline]
    pub fn is_superset(&self, other: &LimitHashSet<T>) -> bool {
        other.is_subset(self)
    }

    /// Adds a value to the set and removes the oldest value if the limit is reached.
    ///
    /// If the set did not have this value present, `true` is returned.
    ///
    /// If the set did have this value present, `false` is returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    ///
    /// let mut set = LimitHashSet::new(10);
    ///
    /// assert_eq!(set.insert(2), true);
    /// assert_eq!(set.insert(2), false);
    /// assert_eq!(set.len(), 1);
    /// ```
    pub fn insert(&mut self, value: T) -> bool {
        let not_present = self.list.remove(&value).is_none();
        self.list.push_back(value);
        if self.len() > self.limit {
            self.list.pop_front();
        }
        not_present
    }

    /// Adds a value to the set, replacing the existing value, if any, that is equal to the given
    /// one. Returns the replaced value. Removes the oldest value if the limit is reached.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    ///
    /// let mut set = LimitHashSet::new(10);
    /// set.insert(Vec::<i32>::new());
    ///
    /// assert_eq!(set.get(&Vec::<i32>::new()).unwrap().capacity(), 0);
    /// set.replace(Vec::with_capacity(10));
    /// assert_eq!(set.get(&Vec::<i32>::new()).unwrap().capacity(), 10);
    /// ```
    pub fn replace(&mut self, value: T) -> Option<T> {
        let previous = self.list.remove(&value);
        self.list.push_back(value);
        if self.len() > self.limit {
            self.list.pop_front();
        }
        previous
    }

    /// Adds a value to the set, replacing the existing value, if any, that is equal to the given
    /// one. Removes and returns the oldest value if the limit is reached.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    ///
    /// let mut set = LimitHashSet::new(2);
    /// assert_eq!(set.insert_and_get_removed(4), None);
    /// assert_eq!(set.insert_and_get_removed(3), None);
    /// assert_eq!(set.insert_and_get_removed(5), Some(4));
    /// ```
    pub fn insert_and_get_removed(&mut self, value: T) -> Option<T> {
        self.list.remove(&value);
        self.list.push_back(value);
        if self.len() > self.limit {
            return self.list.pop_front();
        }
        None
    }

    /// Removes a value from the set. Returns `true` if the value was
    /// present in the set.
    ///
    /// The value may be any borrowed form of the set's value type, but
    /// `Hash` and `Eq` on the borrowed form *must* match those for
    /// the value type.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    ///
    /// let mut set = LimitHashSet::new(10);
    ///
    /// set.insert(2);
    /// assert_eq!(set.remove(&2), true);
    /// assert_eq!(set.remove(&2), false);
    /// ```
    pub fn remove<Q: ?Sized>(&mut self, value: &Q) -> bool
        where Rc<T>: Borrow<Q>,
              Q: Hash + Eq
    {
        self.list.remove(value).is_some()
    }

    /// Removes and returns the value in the set, if any, that is equal to the given one.
    ///
    /// The value may be any borrowed form of the set's value type, but
    /// `Hash` and `Eq` on the borrowed form *must* match those for
    /// the value type.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    ///
    /// let mut set: LimitHashSet<_> = [1, 2, 3].iter().cloned().collect();
    /// assert_eq!(set.take(&2), Some(2));
    /// assert_eq!(set.take(&2), None);
    /// ```
    pub fn take<Q: ?Sized>(&mut self, value: &Q) -> Option<T>
        where Rc<T>: Borrow<Q>,
              Q: Hash + Eq
    {
        self.list.remove(value)
    }
}

impl<T> PartialEq for LimitHashSet<T>
    where T: Eq + Hash
{
    fn eq(&self, other: &LimitHashSet<T>) -> bool {
        if self.len() != other.len() {
            return false;
        }

        self.iter().all(|key| other.contains(key))
    }
}

impl<T> Eq for LimitHashSet<T>
    where T: Eq + Hash
{
}

impl<T> fmt::Debug for LimitHashSet<T>
    where T: Eq + Hash + fmt::Debug
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

impl<T> FromIterator<T> for LimitHashSet<T>
    where T: Eq + Hash
{
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> LimitHashSet<T> {
        let iter = iter.into_iter();
        let (min_size, max_size) = iter.size_hint();
        let size = match max_size {
            Some(size) => size,
            None => min_size,
        };
        let mut set = LimitHashSet::new(size);
        set.extend(iter);
        set
    }
}

impl<T> Extend<T> for LimitHashSet<T>
    where T: Eq + Hash
{
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for elt in iter {
            self.insert(elt);
        }
    }
}

impl<'a, T> Extend<&'a T> for LimitHashSet<T>
    where T: 'a + Eq + Hash + Copy
{
    fn extend<I: IntoIterator<Item = &'a T>>(&mut self, iter: I) {
        self.extend(iter.into_iter().cloned());
    }
}

impl<'a, 'b, T> BitOr<&'b LimitHashSet<T>> for &'a LimitHashSet<T>
    where T: Eq + Hash + Clone
{
    type Output = LimitHashSet<T>;

    /// Returns the union of `self` and `rhs` as a new `LimitHashSet<T>`.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    ///
    /// let a: LimitHashSet<_> = vec![1, 2, 3].into_iter().collect();
    /// let b: LimitHashSet<_> = vec![3, 4, 5].into_iter().collect();
    ///
    /// let set = &a | &b;
    ///
    /// let mut i = 0;
    /// let expected = [1, 2, 3, 4, 5];
    /// for x in &set {
    ///     assert!(expected.contains(x));
    ///     i += 1;
    /// }
    /// assert_eq!(i, expected.len());
    /// ```
    fn bitor(self, rhs: &LimitHashSet<T>) -> LimitHashSet<T> {
        self.union(rhs).cloned().collect()
    }
}

impl<'a, 'b, T> BitAnd<&'b LimitHashSet<T>> for &'a LimitHashSet<T>
    where T: Eq + Hash + Clone
{
    type Output = LimitHashSet<T>;

    /// Returns the intersection of `self` and `rhs` as a new `LimitHashSet<T>`.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    ///
    /// let a: LimitHashSet<_> = vec![1, 2, 3].into_iter().collect();
    /// let b: LimitHashSet<_> = vec![2, 3, 4].into_iter().collect();
    ///
    /// let set = &a & &b;
    ///
    /// let mut i = 0;
    /// let expected = [2, 3];
    /// for x in &set {
    ///     assert!(expected.contains(x));
    ///     i += 1;
    /// }
    /// assert_eq!(i, expected.len());
    /// ```
    fn bitand(self, rhs: &LimitHashSet<T>) -> LimitHashSet<T> {
        self.intersection(rhs).cloned().collect()
    }
}

impl<'a, 'b, T> BitXor<&'b LimitHashSet<T>> for &'a LimitHashSet<T>
    where T: Eq + Hash + Clone
{
    type Output = LimitHashSet<T>;

    /// Returns the symmetric difference of `self` and `rhs` as a new `LimitHashSet<T>`.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    ///
    /// let a: LimitHashSet<_> = vec![1, 2, 3].into_iter().collect();
    /// let b: LimitHashSet<_> = vec![3, 4, 5].into_iter().collect();
    ///
    /// let set = &a ^ &b;
    ///
    /// let mut i = 0;
    /// let expected = [1, 2, 4, 5];
    /// for x in &set {
    ///     assert!(expected.contains(x));
    ///     i += 1;
    /// }
    /// assert_eq!(i, expected.len());
    /// ```
    fn bitxor(self, rhs: &LimitHashSet<T>) -> LimitHashSet<T> {
        self.symmetric_difference(rhs).cloned().collect()
    }
}

impl<'a, 'b, T> Sub<&'b LimitHashSet<T>> for &'a LimitHashSet<T>
    where T: Eq + Hash + Clone
{
    type Output = LimitHashSet<T>;

    /// Returns the difference of `self` and `rhs` as a new `LimitHashSet<T>`.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    ///
    /// let a: LimitHashSet<_> = vec![1, 2, 3].into_iter().collect();
    /// let b: LimitHashSet<_> = vec![3, 4, 5].into_iter().collect();
    ///
    /// let set = &a - &b;
    ///
    /// let mut i = 0;
    /// let expected = [1, 2];
    /// for x in &set {
    ///     assert!(expected.contains(x));
    ///     i += 1;
    /// }
    /// assert_eq!(i, expected.len());
    /// ```
    fn sub(self, rhs: &LimitHashSet<T>) -> LimitHashSet<T> {
        self.difference(rhs).cloned().collect()
    }
}

/// A lazy iterator producing elements in the intersection of `LimitHashSet`s.
///
/// This `struct` is created by the [`intersection`] method on [`LimitHashSet`].
/// See its documentation for more.
///
/// [`LimitHashSet`]: struct.LimitHashSet.html
/// [`intersection`]: struct.LimitHashSet.html#method.intersection
pub struct Intersection<'a, T: 'a>
    where T: Hash + Eq {
    // iterator of the first set
    iter: Iter<'a, T>,
    // the second set
    other: &'a LimitHashSet<T>,
}

/// A lazy iterator producing elements in the difference of `LimitHashSet`s.
///
/// This `struct` is created by the [`difference`] method on [`LimitHashSet`].
/// See its documentation for more.
///
/// [`LimitHashSet`]: struct.LimitHashSet.html
/// [`difference`]: struct.LimitHashSet.html#method.difference
pub struct Difference<'a, T: 'a>
    where T: Hash + Eq {
    // iterator of the first set
    iter: Iter<'a, T>,
    // the second set
    other: &'a LimitHashSet<T>,
}

/// A lazy iterator producing elements in the symmetric difference of `LimitHashSet`s.
///
/// This `struct` is created by the [`symmetric_difference`] method on
/// [`LimitHashSet`]. See its documentation for more.
///
/// [`LimitHashSet`]: struct.LimitHashSet.html
/// [`symmetric_difference`]: struct.LimitHashSet.html#method.symmetric_difference
pub struct SymmetricDifference<'a, T: 'a>
    where T: Hash + Eq {
    iter: Chain<Difference<'a, T>, Difference<'a, T>>,
}

/// A lazy iterator producing elements in the union of `LimitHashSet`s.
///
/// This `struct` is created by the [`union`] method on [`LimitHashSet`].
/// See its documentation for more.
///
/// [`LimitHashSet`]: struct.LimitHashSet.html
/// [`union`]: struct.LimitHashSet.html#method.union
pub struct Union<'a, T: 'a>
    where T: Hash + Eq {
    iter: Chain<Iter<'a, T>, Difference<'a, T>>,
}

impl<'a, T> IntoIterator for &'a LimitHashSet<T>
    where T: Eq + Hash
{
    type Item = &'a T;
    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Iter<'a, T> {
        self.iter()
    }
}

impl<T> IntoIterator for LimitHashSet<T>
    where T: Eq + Hash
{
    type Item = T;
    type IntoIter = IntoIter<T>;

    /// Creates a consuming iterator, that is, one that moves each value out
    /// of the set in insertion order. The set cannot be used after calling
    /// this.
    ///
    /// # Examples
    ///
    /// ```
    /// use extended_collections::LimitHashSet;
    /// let mut set = LimitHashSet::new(10);
    /// set.insert("a".to_string());
    /// set.insert("b".to_string());
    ///
    /// // Not possible to collect to a Vec<String> with a regular `.iter()`.
    /// let v: Vec<String> = set.into_iter().collect();
    ///
    /// // Will print in an arbitrary order.
    /// for x in &v {
    ///     println!("{}", x);
    /// }
    /// ```
    fn into_iter(self) -> IntoIter<T> {
        self.list.into_iter()
    }
}

impl<'a, T> Clone for Intersection<'a, T>
    where T: Eq + Hash + Clone {
    fn clone(&self) -> Intersection<'a, T> {
        Intersection { iter: self.iter.clone(), ..*self }
    }
}

impl<'a, T> Iterator for Intersection<'a, T>
    where T: Eq + Hash
{
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        loop {
            let elt = self.iter.next()?;
            if self.other.contains(elt) {
                return Some(elt);
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let (_, upper) = self.iter.size_hint();
        (0, upper)
    }
}

impl<'a, T> fmt::Debug for Intersection<'a, T>
    where T: fmt::Debug + Eq + Hash + Clone
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

impl<'a, T> FusedIterator for Intersection<'a, T>
    where T: Eq + Hash
{
}

impl<'a, T> Clone for Difference<'a, T>
    where T: Eq + Hash + Clone {
    fn clone(&self) -> Difference<'a, T> {
        Difference { iter: self.iter.clone(), ..*self }
    }
}

impl<'a, T> Iterator for Difference<'a, T>
    where T: Eq + Hash
{
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        loop {
            let elt = self.iter.next()?;
            if !self.other.contains(elt) {
                return Some(elt);
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let (_, upper) = self.iter.size_hint();
        (0, upper)
    }
}

impl<'a, T> FusedIterator for Difference<'a, T>
    where T: Eq + Hash
{
}

impl<'a, T> fmt::Debug for Difference<'a, T>
    where T: fmt::Debug + Eq + Hash + Clone
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

impl<'a, T> Clone for SymmetricDifference<'a, T>
    where T: Eq + Hash + Clone {
    fn clone(&self) -> SymmetricDifference<'a, T> {
        SymmetricDifference { iter: self.iter.clone() }
    }
}

impl<'a, T> Iterator for SymmetricDifference<'a, T>
    where T: Eq + Hash
{
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        self.iter.next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<'a, T> FusedIterator for SymmetricDifference<'a, T>
    where T: Eq + Hash
{
}

impl<'a, T> fmt::Debug for SymmetricDifference<'a, T>
    where T: fmt::Debug + Eq + Hash + Clone
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

impl<'a, T> Clone for Union<'a, T>
    where T: Eq + Hash + Clone {
    fn clone(&self) -> Union<'a, T> {
        Union { iter: self.iter.clone() }
    }
}

impl<'a, T> FusedIterator for Union<'a, T>
    where T: Eq + Hash
{
}

impl<'a, T> fmt::Debug for Union<'a, T>
    where T: fmt::Debug + Eq + Hash + Clone
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

impl<'a, T> Iterator for Union<'a, T>
    where T: Eq + Hash
{
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        self.iter.next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

#[allow(dead_code)]
fn assert_covariance() {
    fn set<'new>(v: LimitHashSet<&'static str>) -> LimitHashSet<&'new str> {
        v
    }
    fn difference<'a, 'new>(v: Difference<'a, &'static str>)
                            -> Difference<'a, &'new str> {
        v
    }
    fn symmetric_difference<'a, 'new>(v: SymmetricDifference<'a, &'static str>)
                                      -> SymmetricDifference<'a, &'new str> {
        v
    }
    fn intersection<'a, 'new>(v: Intersection<'a, &'static str>)
                              -> Intersection<'a, &'new str> {
        v
    }
    fn union<'a, 'new>(v: Union<'a, &'static str>)
                       -> Union<'a, &'new str> {
        v
    }
}

#[cfg(test)]
mod test_set {
    use std::hash;

    use super::LimitHashSet;

    #[test]
    fn test_disjoint() {
        let mut xs = LimitHashSet::new(10);
        let mut ys = LimitHashSet::new(10);
        assert!(xs.is_disjoint(&ys));
        assert!(ys.is_disjoint(&xs));
        assert!(xs.insert(5));
        assert!(ys.insert(11));
        assert!(xs.is_disjoint(&ys));
        assert!(ys.is_disjoint(&xs));
        assert!(xs.insert(7));
        assert!(xs.insert(19));
        assert!(xs.insert(4));
        assert!(ys.insert(2));
        assert!(ys.insert(-11));
        assert!(xs.is_disjoint(&ys));
        assert!(ys.is_disjoint(&xs));
        assert!(ys.insert(7));
        assert!(!xs.is_disjoint(&ys));
        assert!(!ys.is_disjoint(&xs));
    }

    #[test]
    fn test_subset_and_superset() {
        let mut a = LimitHashSet::new(10);
        assert!(a.insert(0));
        assert!(a.insert(5));
        assert!(a.insert(11));
        assert!(a.insert(7));

        let mut b = LimitHashSet::new(10);
        assert!(b.insert(0));
        assert!(b.insert(7));
        assert!(b.insert(19));
        assert!(b.insert(250));
        assert!(b.insert(11));
        assert!(b.insert(200));

        assert!(!a.is_subset(&b));
        assert!(!a.is_superset(&b));
        assert!(!b.is_subset(&a));
        assert!(!b.is_superset(&a));

        assert!(b.insert(5));

        assert!(a.is_subset(&b));
        assert!(!a.is_superset(&b));
        assert!(!b.is_subset(&a));
        assert!(b.is_superset(&a));
    }

    #[test]
    fn test_iterate() {
        let mut a = LimitHashSet::new(32);
        for i in 0..32 {
            assert!(a.insert(i));
        }
        let mut observed: u32 = 0;
        for k in &a {
            observed |= 1 << *k;
        }
        assert_eq!(observed, 0xFFFF_FFFF);
    }

    #[test]
    fn test_intersection() {
        let mut a = LimitHashSet::new(10);
        let mut b = LimitHashSet::new(10);

        assert!(a.insert(11));
        assert!(a.insert(1));
        assert!(a.insert(3));
        assert!(a.insert(77));
        assert!(a.insert(103));
        assert!(a.insert(5));
        assert!(a.insert(-5));

        assert!(b.insert(2));
        assert!(b.insert(11));
        assert!(b.insert(77));
        assert!(b.insert(-9));
        assert!(b.insert(-42));
        assert!(b.insert(5));
        assert!(b.insert(3));

        let mut i = 0;
        let expected = [3, 5, 11, 77];
        for x in a.intersection(&b) {
            assert!(expected.contains(x));
            i += 1
        }
        assert_eq!(i, expected.len());
    }

    #[test]
    fn test_difference() {
        let mut a = LimitHashSet::new(10);
        let mut b = LimitHashSet::new(10);

        assert!(a.insert(1));
        assert!(a.insert(3));
        assert!(a.insert(5));
        assert!(a.insert(9));
        assert!(a.insert(11));

        assert!(b.insert(3));
        assert!(b.insert(9));

        let mut i = 0;
        let expected = [1, 5, 11];
        for x in a.difference(&b) {
            assert!(expected.contains(x));
            i += 1
        }
        assert_eq!(i, expected.len());
    }

    #[test]
    fn test_symmetric_difference() {
        let mut a = LimitHashSet::new(10);
        let mut b = LimitHashSet::new(10);

        assert!(a.insert(1));
        assert!(a.insert(3));
        assert!(a.insert(5));
        assert!(a.insert(9));
        assert!(a.insert(11));

        assert!(b.insert(-2));
        assert!(b.insert(3));
        assert!(b.insert(9));
        assert!(b.insert(14));
        assert!(b.insert(22));

        let mut i = 0;
        let expected = [-2, 1, 5, 11, 14, 22];
        for x in a.symmetric_difference(&b) {
            assert!(expected.contains(x));
            i += 1
        }
        assert_eq!(i, expected.len());
    }

    #[test]
    fn test_union() {
        let mut a = LimitHashSet::new(10);
        let mut b = LimitHashSet::new(10);

        assert!(a.insert(1));
        assert!(a.insert(3));
        assert!(a.insert(5));
        assert!(a.insert(9));
        assert!(a.insert(11));
        assert!(a.insert(16));
        assert!(a.insert(19));
        assert!(a.insert(24));

        assert!(b.insert(-2));
        assert!(b.insert(1));
        assert!(b.insert(5));
        assert!(b.insert(9));
        assert!(b.insert(13));
        assert!(b.insert(19));

        let mut i = 0;
        let expected = [-2, 1, 3, 5, 9, 11, 13, 16, 19, 24];
        for x in a.union(&b) {
            assert!(expected.contains(x));
            i += 1
        }
        assert_eq!(i, expected.len());
    }

    #[test]
    fn test_from_iter() {
        let xs = [1, 2, 3, 4, 5, 6, 7, 8, 9];

        let set: LimitHashSet<_> = xs.iter().cloned().collect();

        for x in &xs {
            assert!(set.contains(x));
        }
    }

    #[test]
    fn test_move_iter() {
        let hs = {
            let mut hs = LimitHashSet::new(10);

            hs.insert('a');
            hs.insert('b');

            hs
        };

        let v = hs.into_iter().collect::<Vec<char>>();
        assert!(v == ['a', 'b'] || v == ['b', 'a']);
    }

    #[test]
    fn test_eq() {
        // These constants once happened to expose a bug in insert().
        // I'm keeping them around to prevent a regression.
        let mut s1 = LimitHashSet::new(10);

        s1.insert(1);
        s1.insert(2);
        s1.insert(3);

        let mut s2 = LimitHashSet::new(10);

        s2.insert(1);
        s2.insert(2);

        assert!(s1 != s2);

        s2.insert(3);

        assert_eq!(s1, s2);
    }

    #[test]
    fn test_show() {
        let mut set = LimitHashSet::new(10);
        let empty = LimitHashSet::<i32>::new(10);

        set.insert(1);
        set.insert(2);

        let set_str = format!("{:?}", set);

        assert!(set_str == "{1, 2}" || set_str == "{2, 1}");
        assert_eq!(format!("{:?}", empty), "{}");
    }

    #[test]
    fn test_replace() {
        #[derive(Debug)]
        struct Foo(&'static str, i32);

        impl PartialEq for Foo {
            fn eq(&self, other: &Self) -> bool {
                self.0 == other.0
            }
        }

        impl Eq for Foo {}

        impl hash::Hash for Foo {
            fn hash<H: hash::Hasher>(&self, h: &mut H) {
                self.0.hash(h);
            }
        }

        let mut s = LimitHashSet::new(10);
        assert_eq!(s.replace(Foo("a", 1)), None);
        assert_eq!(s.len(), 1);
        assert_eq!(s.replace(Foo("a", 2)), Some(Foo("a", 1)));
        assert_eq!(s.len(), 1);

        let mut it = s.iter();
        assert_eq!(it.next(), Some(&Foo("a", 2)));
        assert_eq!(it.next(), None);
    }

    #[test]
    fn test_extend_ref() {
        let mut a = LimitHashSet::new(10);
        a.insert(1);

        a.extend(&[2, 3, 4]);

        assert_eq!(a.len(), 4);
        assert!(a.contains(&1));
        assert!(a.contains(&2));
        assert!(a.contains(&3));
        assert!(a.contains(&4));

        let mut b = LimitHashSet::new(10);
        b.insert(5);
        b.insert(6);

        a.extend(&b);

        assert_eq!(a.len(), 6);
        assert!(a.contains(&1));
        assert!(a.contains(&2));
        assert!(a.contains(&3));
        assert!(a.contains(&4));
        assert!(a.contains(&5));
        assert!(a.contains(&6));
    }

    #[test]
    fn test_element_limit() {
        let mut set = LimitHashSet::new(2);
        set.extend(vec![1, 2, 3]);

        assert_eq!(set.is_empty(), false);
        assert_eq!(set.len(), 2);
        assert_eq!(set, vec![2, 3].into_iter().collect());

        assert_eq!(set.insert_and_get_removed(4), Some(2));
        assert_eq!(set.remove(&1), false);

        assert_eq!(set.is_empty(), false);
        assert_eq!(set.len(), 2);
        assert_eq!(set, vec![3, 4].into_iter().collect());

        assert_eq!(set.remove(&4), true);

        assert_eq!(set.is_empty(), false);
        assert_eq!(set.len(), 1);
        assert_eq!(set, vec![3].into_iter().collect());

        set.clear();

        assert_eq!(set.is_empty(), true);
        assert_eq!(set.len(), 0);
    }

    #[test]
    #[should_panic]
    fn test_zero_limit() {
        let _set: LimitHashSet<usize> = LimitHashSet::new(0);
    }
}