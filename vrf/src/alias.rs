#![allow(non_snake_case)]

use std::cmp::{Ord, Ordering, PartialOrd};
use std::fmt::Debug;
use std::ops::{Add, Mul, Sub};

use num_traits::sign::Unsigned;
use num_traits::{FromPrimitive, ToPrimitive};

use crate::rng::Rng;

pub struct AliasMethod<P>
where
    P: Copy
        + Debug
        + Unsigned
        + Add<P>
        + Sub<P>
        + Mul<P>
        + FromPrimitive
        + ToPrimitive
        + PartialOrd<P>
        + Ord,
{
    /// The total probability - since we work with integers, this is not 1.0, but corresponds to
    /// a the probability 1.0
    T: P,

    /// Number of entries
    n: usize,

    /// Alias table
    K: Vec<usize>,

    /// Probabilities table
    U: Vec<P>,
}

impl<P> AliasMethod<P>
where
    P: Copy
        + Debug
        + Unsigned
        + Add<P>
        + Sub<P>
        + Mul<P>
        + FromPrimitive
        + ToPrimitive
        + PartialOrd<P>
        + Ord,
{
    pub fn new<V: AsRef<[P]>>(p: V) -> Self {
        // The algorithm was roughly taken from
        //
        // * https://en.wikipedia.org/wiki/Alias_method#Table_generation
        // * https://github.com/asmith26/Vose-Alias-Method/blob/master/vose_sampler/vose_sampler.py
        //
        // p - probabilities p_i. We will use this for U as well
        // T - total probability
        // n - number of probabilities

        let p = p.as_ref();
        let n = p.len();

        // Construct scaled probabilities and total probability.
        let mut T = P::zero();

        let mut U: Vec<P> = p
            .iter()
            .map(|p| {
                T = T + *p;
                p.mul(P::from_usize(n).expect("Can't convert n to P for normalization"))
            })
            .collect();

        // Construct overfull and underfull stack. These contain only indices into U.
        let mut U_underfull = Vec::with_capacity(n);
        let mut U_overfull = Vec::with_capacity(n);

        for (i, U_i) in U.iter().enumerate() {
            match U_i.cmp(&T) {
                Ordering::Equal => (),
                Ordering::Greater => U_overfull.push(i),
                Ordering::Less => U_underfull.push(i),
            }
        }

        // Construct alias table.
        let mut K: Vec<usize> = (0..n).collect();

        while let (Some(i_u), Some(i_o)) = (U_underfull.pop(), U_overfull.pop()) {
            // Alias overfull into underfull.
            K[i_u] = i_o;

            // Remove allocated space from U: U_o -= (T - U_u)
            U[i_o] = U[i_o] + U[i_u] - T;

            // Assign entry i_o to the appropriate category base on the new value.
            match U[i_o].cmp(&T) {
                Ordering::Equal => (),
                Ordering::Greater => U_overfull.push(i_o),
                Ordering::Less => U_underfull.push(i_o),
            }
        }

        // Both must be empty now.
        debug_assert!(U_underfull.is_empty() && U_overfull.is_empty());

        // Entries that are "underfull" need an entry in the alias table.
        debug_assert!((0..n).all(|i| {
            // Both must be true or both must be false.
            (U[i] < T) == (K[i] != i)
        }));

        Self { T, n, K, U }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    pub fn total(&self) -> P {
        self.T
    }

    /// Sample from the discrete random distribution
    ///
    /// # Arguments
    ///
    /// * `x`: Must be uniformly random between 0 and `n`
    /// * `y`: Must be uniformly random between 0 and `T`
    ///
    /// # Return value
    ///
    /// Returns the index corresponding to the probability in the input `p`.
    ///
    pub fn sample<R: Rng>(&self, rng: &mut R) -> usize {
        let x = rng.next_u64_max(self.n as u64) as usize;

        let y = P::from_u64(rng.next_u64_max(self.T.to_u64().unwrap())).unwrap();

        if y < self.U[x] {
            x
        } else {
            self.K[x]
        }
    }
}
