use crate::{FixedUnsigned, FixedScale};



/*macro_rules! create_typed_array {
    ($scale: expr) => {
    /// A fixed point Uint with 16 decimal places
        pub struct FixedScale$scale {}
        impl ::fixed_unsigned::FixedScale for FixedScale$scale {
            const SCALE: u64 = 16;
        }
        pub type FixedUnsigned$scale = FixedUnsigned<FixedScale$scale>;
    };
}*/


/// A fixed point Uint with 4 decimal places
pub struct FixedScale4 {}
impl FixedScale for FixedScale4 {
    const SCALE: u64 = 4;
}
pub type FixedUnsigned4 = FixedUnsigned<FixedScale4>;


/// A fixed point Uint with 8 decimal places
pub struct FixedScale8 {}
impl FixedScale for FixedScale8 {
    const SCALE: u64 = 8;
}
pub type FixedUnsigned8 = FixedUnsigned<FixedScale8>;


/// A fixed point Uint with 16 decimal places
pub struct FixedScale16 {}
impl FixedScale for FixedScale16 {
    const SCALE: u64 = 16;
}
pub type FixedUnsigned16 = FixedUnsigned<FixedScale16>;
