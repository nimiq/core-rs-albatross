// Required for LinkedList.
#![feature(box_into_raw_non_null)]
#![feature(specialization)]
#![feature(map_get_key_value)]

extern crate beserial;
#[macro_use]
extern crate beserial_derive;

pub mod linked_list;
pub mod unique_linked_list;
pub mod queue;
pub mod limit_hash_set;
pub mod sparse_vec;
pub mod segment_tree;
pub mod grouped_list;


#[cfg(feature = "bitset")]
pub mod bitset;
#[cfg(feature = "bitset")]
pub mod compressed_list;


pub use self::linked_list::LinkedList;
pub use self::unique_linked_list::UniqueLinkedList;
pub use self::queue::Queue;
pub use self::limit_hash_set::LimitHashSet;
pub use self::sparse_vec::SparseVec;
pub use self::segment_tree::SegmentTree;
