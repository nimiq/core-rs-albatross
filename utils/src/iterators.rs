use std::iter::Peekable;
use std::cmp::Ordering;

/// An iterator that alternates between two iterators.
///
/// [`Iterator`]: trait.Iterator.html
#[derive(Clone, Debug)]
pub struct Alternate<A, B> {
    a: A,
    b: B,
    state: ChainState,
}

impl<A, B> Alternate<A, B> {
    pub fn new(a: A, b: B) -> Self {
        Self {
            a,
            b,
            state: ChainState::BothA,
        }
    }
}

// The iterator protocol specifies that iteration ends with the return value
// `None` from `.next()` (or `.next_back()`) and it is unspecified what
// further calls return. The chain adaptor must account for this since it uses
// two subiterators.
//
//  It uses four states:
//
//  - BothA: `a` and `b` are remaining, next turn is `a`
//  - BothB: `a` and `b` are remaining, next turn is `b`
//  - OnlyA: `a` remaining
//  - OnlyB: `b` remaining
//
//  The fifth state (neither iterator is remaining) only occurs after Chain has
//  returned None once, so we don't need to store this state.
#[derive(Clone, Debug)]
enum ChainState {
    // both `a` and `b` iterator are remaining, next turn is `a`
    BothA,
    // both `a` and `b` iterator are remaining, next turn is `b`
    BothB,
    // only `a` is remaining
    OnlyA,
    // only `b` is remaining
    OnlyB,
}

impl<A, B> Iterator for Alternate<A, B> where
    A: Iterator,
    B: Iterator<Item = A::Item>
{
    type Item = A::Item;

    #[inline]
    fn next(&mut self) -> Option<A::Item> {
        match self.state {
            ChainState::BothA => match self.a.next() {
                elt @ Some(..) => {
                    self.state = ChainState::BothB;
                    elt
                },
                None => {
                    self.state = ChainState::OnlyB;
                    self.b.next()
                }
            },
            ChainState::BothB => match self.b.next() {
                elt @ Some(..) => {
                    self.state = ChainState::BothA;
                    elt
                },
                None => {
                    self.state = ChainState::OnlyA;
                    self.a.next()
                }
            },
            ChainState::OnlyA => self.a.next(),
            ChainState::OnlyB => self.b.next(),
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let (a_lower, a_upper) = self.a.size_hint();
        let (b_lower, b_upper) = self.b.size_hint();

        let lower = a_lower.saturating_add(b_lower);

        let upper = match (a_upper, b_upper) {
            (Some(x), Some(y)) => x.checked_add(y),
            _ => None
        };

        (lower, upper)
    }

    #[inline]
    fn count(self) -> usize {
        match self.state {
            ChainState::BothA | ChainState::BothB => self.a.count() + self.b.count(),
            ChainState::OnlyA => self.a.count(),
            ChainState::OnlyB => self.b.count(),
        }
    }
}

/// An iterator that merges two iterators, removing duplicates.
pub struct Merge<L, R, C> where
    L: Iterator,
    R: Iterator<Item = L::Item>,
    C: Fn(&L::Item, &R::Item) -> Ordering,
{
    left: Peekable<L>,
    right: Peekable<R>,
    cmp: C,
}

impl<L, R, C> Merge<L, R, C> where
    L: Iterator,
    R: Iterator<Item = L::Item>,
    C: Fn(&L::Item, &R::Item) -> Ordering,
{
    pub fn new(left: L, right: R, cmp: C) -> Self {
        Merge {
            left: left.peekable(),
            right: right.peekable(),
            cmp,
        }
    }
}

impl<L, R, C> Iterator for Merge<L, R, C> where
    L: Iterator,
    R: Iterator<Item = L::Item>,
    C: Fn(&L::Item, &R::Item) -> Ordering,
{
    type Item = L::Item;

    fn next(&mut self) -> Option<Self::Item> {
        let ordering = match (self.left.peek(), self.right.peek()) {
            (Some(l), Some(r))  => Some((self.cmp)(l, r)),
            (Some(_), None)     => Some(Ordering::Less),
            (None, Some(_))     => Some(Ordering::Greater),
            (None, None)        => None,
        };

        match ordering {
            Some(Ordering::Less)    => self.left.next(),
            Some(Ordering::Greater) => self.right.next(),
            Some(Ordering::Equal)   => { self.left.next(); self.right.next() },
            None                    => None,
        }
    }
}
