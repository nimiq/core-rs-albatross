use crate::mmr::position::{index_to_height, Position};

pub(crate) struct PeakIterator {
    size: usize,
    current_peak: Option<Position>,
}

impl PeakIterator {
    pub(crate) fn new(size: usize) -> Self {
        let current_peak = if size == 0 {
            None
        } else {
            // For non-empty trees, the first peak is at position 2^n - 2 such that this is
            // the largest n for which 2^n - 1 <= size.
            // The following code uses k = 2^n.
            let mut k = 1;
            // Find largest k such that k - 1 <= size.
            while k - 1 <= size {
                k <<= 1;
            }
            // Calculate peak index when starting with index 1.
            let peak_index_one_based = (k >> 1) - 1;
            Some(Position {
                index: peak_index_one_based - 1, // Convert to 0-based.
                height: peak_index_one_based.count_ones() as usize - 1, // The number of ones - 1 is the height.
                right_node: false, // The first peak is always a left node.
            })
        };

        PeakIterator { size, current_peak }
    }
}

impl Iterator for PeakIterator {
    type Item = Position;

    fn next(&mut self) -> Option<Self::Item> {
        let current_peak = self.current_peak?;

        // Try calculating the next peak.
        // For this, we first take the right sibling of our current peak
        // (invariant: the peaks are always on the left).
        // This node is never part of our MMR (as we could otherwise have another peak on top of it).
        // Then, we move to the left child until we find a position in our tree.
        let sibling = current_peak.sibling();
        // If we are at height 0, the left child does not exist.
        let mut peak_candidate = sibling.left_child();
        // Iterate until we find a left child that is within the MMR's bounds.
        while peak_candidate
            .map(|p| p.index > self.size - 1)
            .unwrap_or(false)
        {
            // We cannot enter this loop if `peak_candidate` is None.
            peak_candidate = peak_candidate.unwrap().left_child();
        }
        self.current_peak = peak_candidate;

        Some(current_peak)
    }
}

pub(crate) struct RevPeakIterator {
    current_peak: Option<Position>,
}

impl RevPeakIterator {
    pub(crate) fn new(size: usize) -> Self {
        let current_peak = if size == 0 {
            None
        } else {
            // For non-empty trees, the last peak is always at the most recent position.
            Some(Position {
                index: size - 1,
                height: index_to_height(size - 1),
                right_node: false, // A peak is always a left node.
            })
        };

        RevPeakIterator { current_peak }
    }
}

impl Iterator for RevPeakIterator {
    type Item = Position;

    fn next(&mut self) -> Option<Self::Item> {
        let current_peak = self.current_peak?;

        // Try calculating the previous peak.
        // Since each peak is a perfect binary tree, the previous peak is always
        // the element just in front of all nodes belonging to the current tree.
        let tree_nodes = (2 << current_peak.height) - 1; // 2^(h+1) - 1

        // If the current index + 1 equals the number of nodes in the current tree,
        // we've reached the last peak.
        if current_peak.index + 1 == tree_nodes {
            self.current_peak = None;
        } else {
            let index = current_peak.index - tree_nodes;
            self.current_peak = Some(Position {
                index,
                height: index_to_height(index),
                right_node: false,
            });
        }

        Some(current_peak)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_correctly_iterates_peaks() {
        fn test_peaks(size: usize, peaks: Vec<usize>) {
            let pi = PeakIterator::new(size);
            let calc_peaks: Vec<Position> = pi.collect();
            let peaks: Vec<Position> = peaks.iter().map(|i| Position::from(*i)).collect();
            assert_eq!(calc_peaks, peaks, "Peaks for size {}", size);

            // Reversed peaks.
            let pi = RevPeakIterator::new(size);
            let calc_peaks: Vec<Position> = pi.collect();
            let peaks: Vec<Position> = peaks.into_iter().rev().collect();
            assert_eq!(calc_peaks, peaks, "Rev Peaks for size {}", size);
        }

        test_peaks(0, vec![]);
        test_peaks(1, vec![0]);
        test_peaks(3, vec![2]);
        test_peaks(4, vec![2, 3]);
        test_peaks(7, vec![6]);
        test_peaks(8, vec![6, 7]);
        test_peaks(10, vec![6, 9]);
        test_peaks(11, vec![6, 9, 10]);
        test_peaks(15, vec![14]);
    }
}
