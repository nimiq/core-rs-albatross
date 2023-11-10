use bitflags::bitflags;

mod error;
pub mod mdbx;
/// Database implementation that can handle volatile and persistent storage.
pub mod proxy;
/// Abstraction for methods related to the database.
pub mod traits;
pub mod volatile;

pub use error::*;
pub use proxy::*;

bitflags! {
    #[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
    pub struct TableFlags: u32 {
        /// Duplicate keys may be used in the database.
        const DUPLICATE_KEYS        = 0b0000_0001;
        /// This flag may only be used in combination with `DUPLICATE_KEYS`.
        /// This option tells the database that the values for this database are all the same size.
        const DUP_FIXED_SIZE_VALUES = 0b0000_0010;
        /// Keys are binary integers in native byte order and will be sorted as such
        /// (`std::os::raw::c_uint`, i.e. most likely `u32`).
        const UINT_KEYS             = 0b0000_0100;
    }
}
