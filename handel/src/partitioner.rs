use std::ops::RangeInclusive;
// use nimiq_collections::bitset::BitSet;

use utils::math::log2;

use crate::contribution::AggregatableContribution;

/// Errors that can happen during partitioning
#[derive(Clone, Debug, Fail, PartialEq)]
pub enum PartitioningError {
    #[fail(display = "Invalid level: {}", level)]
    InvalidLevel { level: usize },
    #[fail(display = "Empty level: {}", level)]
    EmptyLevel { level: usize },
}

pub trait Partitioner: Send + Sync {
    /// Number of levels
    fn levels(&self) -> usize;

    /// Total number of identities
    fn size(&self) -> usize;

    /// Number of identities at `level`
    fn level_size(&self, level: usize) -> usize;

    /// Range of identities that need to be contacted at `level`
    fn range(&self, level: usize) -> Result<RangeInclusive<usize>, PartitioningError>;

    /// Combine `AggregatableContributions` to a new `AggregatableContribution` for next level
    /// TODO: Return `Result<C, PartitioningError>` instead of option
    fn combine<C: AggregatableContribution>(&self, signatures: Vec<&C>, level: usize) -> Option<C>;
}

/// The next level is always double the size of the current level
#[derive(Clone, Debug)]
pub struct BinomialPartitioner {
    /// The ID of the node itself
    node_id: usize,

    /// The number of IDs handled (i.e. `max_id + 1`)
    num_ids: usize,

    /// The number of levels
    num_levels: usize,
}

impl BinomialPartitioner {
    pub fn new(node_id: usize, num_ids: usize) -> Self {
        let num_levels = match num_ids {
            0 => panic!("num_ids must be greater than 0"),
            1 => 1,
            n => log2(n - 1) + 2,
        };
        assert!(node_id < num_ids);
        Self {
            node_id,
            num_ids,
            num_levels,
        }
    }
}

impl Partitioner for BinomialPartitioner {
    /// returns the number of levels including level 0 (which is always this nodes own contribution)
    fn levels(&self) -> usize {
        self.num_levels
    }

    fn size(&self) -> usize {
        self.num_ids
    }

    fn level_size(&self, level: usize) -> usize {
        2_usize.pow(level as u32)
    }

    fn range(&self, level: usize) -> Result<RangeInclusive<usize>, PartitioningError> {
        if level == 0 {
            Ok(self.node_id..=self.node_id)
        } else if level >= self.num_levels {
            Err(PartitioningError::InvalidLevel { level })
        } else {
            // mask for bits which cover the range
            let m = (1 << (level - 1)) - 1;
            // bit that must be flipped
            let f = 1 << (level - 1);

            let min = (self.node_id ^ f) & !m;
            let max = (self.node_id ^ f) | m;

            if min > max {
                Err(PartitioningError::EmptyLevel { level })
            } else {
                Ok(min..=max)
            }
        }
    }

    /// TODO: Why do we have `_level` as argument?
    fn combine<C: AggregatableContribution>(
        &self,
        contributions: Vec<&C>,
        _level: usize,
    ) -> Option<C> {
        let mut combined = (*contributions.first()?).clone();

        for contribution in contributions.iter().skip(1) {
            combined
                .combine(contribution)
                .unwrap_or_else(|e| panic!("Failed to combine contributions: {}", e));
        }

        debug!("Combined signature: {:?}", combined);
        Some(combined)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_partitioner() {
        /*
        partitioner: node_id = 3
            ---ID---   -Level-
            0    000   . . 2 .
            1    001   . . 2 .
            2    010   . 1 . .
            3    011   0 . . .
            4    100   . . . 3
            5    101   . . . 3
            6    110   . . . 3
            7    111   . . . 3

        level = 3
        m = (1 << level - 1) - 1 = 100 - 1 = 011
        f = (1 << level)                   = 100


        other_partitioner: node_id = 0
            ---ID---   -Level-
            0    000   . 1 . .
            1    001   0 . . .
            2    010   . . 2 .
            3    011   . . 2 .
            4    100   . . . 3
            5    101   . . . 3
            6    110   . . . 3
            7    111   . . . 3
        */

        let partitioner = BinomialPartitioner::new(3, 8);
        let second_partitioner = BinomialPartitioner::new(1, 8);
        let third_partitioner = BinomialPartitioner::new(7, 8);

        assert_eq!(partitioner.levels(), 4);

        assert_eq!(partitioner.range(0), Ok(3..=3), "Level 0");
        assert_eq!(second_partitioner.range(0), Ok(1..=1), "Level 0");

        assert_eq!(partitioner.range(1), Ok(2..=2), "Level 1");
        assert_eq!(second_partitioner.range(1), Ok(0..=0), "Level 1");

        assert_eq!(partitioner.range(2), Ok(0..=1), "Level 2");
        assert_eq!(second_partitioner.range(2), Ok(2..=3), "Level 2");

        assert_eq!(partitioner.range(3), Ok(4..=7), "Level 3");
        assert_eq!(second_partitioner.range(3), Ok(4..=7), "Level 3");

        // must be symetrical
        for level in 2..partitioner.levels() {
            if partitioner
                .range(level)
                .unwrap()
                .contains(&second_partitioner.node_id)
            {
                assert!(second_partitioner
                    .range(level)
                    .unwrap()
                    .contains(&partitioner.node_id));
            }
            if partitioner
                .range(level)
                .unwrap()
                .contains(&third_partitioner.node_id)
            {
                assert!(third_partitioner
                    .range(level)
                    .unwrap()
                    .contains(&partitioner.node_id));
            }
        }

        assert_eq!(
            partitioner.range(4),
            Err(PartitioningError::InvalidLevel { level: 4 })
        );
    }

    #[test]
    fn test_non_power_of_two() {
        assert_eq!(BinomialPartitioner::new(0, 7).levels(), 4);
        assert_eq!(BinomialPartitioner::new(0, 6).levels(), 4);
        assert_eq!(BinomialPartitioner::new(0, 5).levels(), 4);
        assert_eq!(BinomialPartitioner::new(0, 4).levels(), 3);
    }
}
