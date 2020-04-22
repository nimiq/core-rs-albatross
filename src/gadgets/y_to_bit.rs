use algebra::PrimeField;
use algebra_core::PairingEngine;
use r1cs_core::SynthesisError;
use r1cs_std::bits::boolean::Boolean;
use r1cs_std::pairing::PairingGadget;

/// A gadget that takes an elliptic curve point as input and outputs a single bit representing the
/// "sign" of the y-coordinate. It is meant to aid with serialization.
/// It was originally part of the Celo light client library. (https://github.com/celo-org/bls-zexe)
pub trait YToBitGadget<
    PairingE: PairingEngine,
    ConstraintF: PrimeField,
    PG: PairingGadget<PairingE, ConstraintF>,
>
{
    /// Outputs a boolean representing the relation:
    /// y > half
    /// where half means the half point of the modulus of the underlying field. So, half = (p-1)/2.
    fn y_to_bit_g1<CS: r1cs_core::ConstraintSystem<ConstraintF>>(
        cs: CS,
        point: &PG::G1Gadget,
    ) -> Result<Boolean, SynthesisError>;

    /// Outputs a boolean representing the relation:
    /// (y_c1 > half) || (y_c1 == 0 && y_c0 > half)
    /// where half means the half point of the modulus of the underlying field. So, half = (p-1)/2.
    fn y_to_bit_g2<CS: r1cs_core::ConstraintSystem<ConstraintF>>(
        cs: CS,
        point: &PG::G2Gadget,
    ) -> Result<Boolean, SynthesisError>;
}
