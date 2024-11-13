use ark_ec::{pairing::Pairing, CurveGroup};
use ark_ff::Field;

pub trait FixedPairing:
    Pairing<BaseField = <<<Self as Pairing>::G1 as CurveGroup>::BaseField as Field>::BasePrimeField>
{
}

impl<T> FixedPairing for T where
    T: Pairing<
        BaseField = <<<Self as Pairing>::G1 as CurveGroup>::BaseField as Field>::BasePrimeField,
    >
{
}
