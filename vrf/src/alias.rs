#![allow(non_snake_case)]

use std::cmp::{Ord, Ordering};

use crate::rng::Rng;

pub struct AliasMethod {
    /// The total probability - since we work with integers, this is not 1.0, but corresponds to
    /// a the probability 1.0
    T: u64,

    /// Number of entries
    n: usize,

    /// Alias table
    K: Vec<usize>,

    /// Probabilities table
    U: Vec<u64>,
}

impl AliasMethod {
    pub fn new(p: &[u64]) -> Self {
        // The algorithm was roughly taken from
        //
        // * https://en.wikipedia.org/w/index.php?title=Alias_method&oldid=918053766#Table_generation
        // * https://github.com/asmith26/Vose-Alias-Method/blob/96bffc45b275f2e867f0eb30af7e8ffaaac44596/vose_sampler/vose_sampler.py
        //
        // p - probabilities p_i. We will use this for U as well
        // T - total probability
        // n - number of probabilities

        let n = p.len();

        // Construct scaled probabilities and total probability.
        let mut T = 0;

        let mut U: Vec<u64> = p
            .iter()
            .map(|p| {
                T += *p;
                p * n as u64
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

    pub fn total(&self) -> u64 {
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

        let y = rng.next_u64_max(self.T);

        if y < self.U[x] {
            x
        } else {
            self.K[x]
        }
    }
}
