pub use nimiq_database_value::{AsDatabaseBytes, FromDatabaseBytes};

/// A key in a database table.
pub trait Key: AsDatabaseBytes + FromDatabaseBytes + Ord + 'static {}
impl<T: AsDatabaseBytes + FromDatabaseBytes + Ord + 'static> Key for T {}
/// A value in a database table.
pub trait Value: AsDatabaseBytes + FromDatabaseBytes + 'static {}
impl<T: AsDatabaseBytes + FromDatabaseBytes + 'static> Value for T {}

/// Dup(licate key) table values consist of a sub key and an actual value.
pub trait DupTableValue: Value {
    /// The subkey type.
    type SubKey: Key;

    /// The value type.
    type Value: Value;

    /// Return the subkey value.
    fn subkey(&self) -> &Self::SubKey;

    /// Return the value.
    fn value(&self) -> &Self::Value;
}

/// A database table.
pub trait Table {
    /// The name of the table.
    const NAME: &'static str;

    /// The table's key type.
    type Key: Key;
    /// The table's value type.
    type Value: Value;
}

/// Regular Key-Value tables.
pub trait RegularTable: Table {}

/// Dup(licate key) tables allow us to have a subkey.
pub trait DupTable: Table {}

/// Examples:
/// - `declare_table!(Test, "test", u32 => u32)` will create a regular table mapping u32 to u32.
/// - `declare_table!(Test, "test", u32 => u32 => u32)` will create a dup table mapping u32 to a subkey of u32 to a value of u32.
/// - `declare_table!(Test, "test", u32 => dup(Foo))` will create a dup table mapping u32 to Foo.
#[macro_export]
macro_rules! declare_table {
    // Basic struct, this is not to be used directly.
    ($typ:ident, $name:expr, $key:ty, $value:ty) => {
        #[derive(Clone, Debug, Default)]
        pub struct $typ;
        impl $crate::traits::Table for $typ {
            const NAME: &'static str = $name;
            type Key = $key;
            type Value = $value;
        }
    };
    // Dup table.
    ($typ:ident, $name:expr, $key:ty => dup($value:ty)) => {
        $crate::declare_table!($typ, $name, $key, $value);
        impl $crate::traits::DupTable for $typ {}
    };
    // Regular table.
    ($typ:ident, $name:expr, $key:ty => $value:ty) => {
        $crate::declare_table!($typ, $name, $key, $value);
        impl $crate::traits::RegularTable for $typ {}
    };
    // Indexed table.
    ($typ:ident, $name:expr, $key:ty => $subkey:ty => $value:ty) => {
        $crate::declare_table!($typ, $name, $key, $crate::utils::IndexedValue<$subkey, $value>);
        impl $crate::traits::DupTable for $typ {}
    };
}
