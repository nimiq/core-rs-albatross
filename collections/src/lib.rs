// Required for LinkedList.
#![feature(box_into_raw_non_null)]
#![feature(specialization)]

extern crate beserial;

pub mod linked_list;
pub mod unique_linked_list;
pub mod queue;
pub mod limit_hash_set;
pub mod sparse_vec;
pub mod segment_tree;
#[cfg(feature = "bitset")]
pub mod bitset;

pub use self::linked_list::LinkedList;
pub use self::unique_linked_list::UniqueLinkedList;
pub use self::queue::Queue;
pub use self::limit_hash_set::LimitHashSet;
pub use self::sparse_vec::SparseVec;
pub use self::segment_tree::SegmentTree;
#[cfg(feature = "bitset")]
pub use self::bitset::BitSet;