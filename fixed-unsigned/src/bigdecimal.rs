extern crate bigdecimal;

use crate::{FixedScale, FixedUnsigned};
use bigdecimal::BigDecimal;
use num_bigint::Sign;


trait FromBigDecimal<S>
    where S: FixedScale
{
    fn from_bigdecimal(decimal: BigDecimal) -> Option<FixedUnsigned<S>> {
        if decimal.sign() == Sign::Minus {
            None
        }
        else {
            let (bigint, exponent) = decimal.into_bigint_and_exponent();
            unimplemented!(); // TODO
        }
    }
}

impl<S> From<BigDecimal> for FixedUnsigned<S>
    where S: FixedScale
{
    fn from(int_value: BigUint) -> Self {
        Self::new(Self::scale_up(int_value))
    }
}

