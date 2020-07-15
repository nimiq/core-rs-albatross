use crate::{FixedScale, FixedUnsigned};

/*macro_rules! create_typed_array {
    ($scale: expr) => {
        pub struct FixedScale$scale {}
        impl ::fixed_unsigned::FixedScale for FixedScale$scale {
            const SCALE: u64 = $scale;
        }
        pub type FixedUnsigned$scale = FixedUnsigned<FixedScale$scale>;
    };
}*/

/// A fixed point Uint with 4 decimal places
#[derive(Clone, Debug)]
pub struct FixedScale4 {}

impl FixedScale for FixedScale4 {
    const SCALE: u64 = 4;
}

pub type FixedUnsigned4 = FixedUnsigned<FixedScale4>;

/// A fixed point Uint with 8 decimal places
#[derive(Clone, Debug)]
pub struct FixedScale8 {}

impl FixedScale for FixedScale8 {
    const SCALE: u64 = 8;
}

pub type FixedUnsigned8 = FixedUnsigned<FixedScale8>;

/// A fixed point Uint with 16 decimal places
#[derive(Clone, Debug)]
pub struct FixedScale16 {}

impl FixedScale for FixedScale16 {
    const SCALE: u64 = 16;
}

pub type FixedUnsigned16 = FixedUnsigned<FixedScale16>;

/// A fixed point Uint with 10 decimal places
///
/// NOTE: This should have the same behaviour as bignumber.js (default config with 10 decimal places)
#[derive(Clone, Debug)]
pub struct FixedScale10 {}

impl FixedScale for FixedScale10 {
    const SCALE: u64 = 10;
}

pub type FixedUnsigned10 = FixedUnsigned<FixedScale10>;

/// A fixed point Uint with 10 decimal places
///
/// NOTE: This should have the same behaviour as bignumber.js (default config with 10 decimal places)
#[derive(Clone, Debug)]
pub struct FixedScale26 {}

impl FixedScale for FixedScale26 {
    const SCALE: u64 = 26;
}

pub type FixedUnsigned26 = FixedUnsigned<FixedScale26>;
