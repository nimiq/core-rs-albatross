// Required for LinkedList.
#![feature(specialization)]

#[cfg(feature = "bitset")]
extern crate beserial;

#[cfg(feature = "bitset")]
pub use self::bitset::BitSet;
pub use self::limit_hash_set::LimitHashSet;
pub use self::linked_list::LinkedList;
pub use self::queue::Queue;
pub use self::segment_tree::SegmentTree;
pub use self::sparse_vec::SparseVec;
pub use self::unique_linked_list::UniqueLinkedList;

#[cfg(feature = "bitset")]
pub mod bitset;
pub mod limit_hash_set;
pub mod linked_list;
pub mod queue;
pub mod segment_tree;
pub mod sparse_vec;
pub mod unique_linked_list;
