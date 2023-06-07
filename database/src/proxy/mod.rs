mod cursor;
mod database;
mod transaction;

pub use cursor::*;
pub use database::*;
pub use transaction::*;

use crate::mdbx::MdbxTable;

/// A table handle that is used to reference tables during transactions.
pub type TableProxy = MdbxTable;
