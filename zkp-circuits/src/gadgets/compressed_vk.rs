use std::borrow::Borrow;

use ark_ec::{pairing::Pairing, CurveGroup};
use ark_ff::Field;
use ark_groth16::{constraints::VerifyingKeyVar, VerifyingKey};
use ark_mnt4_753::{constraints::PairingVar as MNT4PairingVar, MNT4_753};
use ark_mnt6_753::{constraints::PairingVar as MNT6PairingVar, MNT6_753};
use ark_r1cs_std::{uint8::UInt8, ToBitsGadget};
use ark_relations::r1cs::{Namespace, SynthesisError};
use nimiq_zkp_primitives::ext_traits::CompressedComposite;

use super::compressed_affine::CompressedAffineVar;

type BasePrimeField<E> = <<<E as Pairing>::G1 as CurveGroup>::BaseField as Field>::BasePrimeField;

pub trait CompressedInput<V, F: Field>
where
    Self: Sized,
    V: ?Sized,
{
    /// Allocates a new variable of type `Self` in the `ConstraintSystem` `cs`.
    /// The mode of allocation is decided by `mode`.
    fn new_compressed_input<T: Borrow<V>>(
        cs: impl Into<Namespace<F>>,
        f: impl FnOnce() -> Result<T, SynthesisError>,
    ) -> Result<Self, SynthesisError>;
}

/// Allocates a verifying key compressed with `CompressedComposite`.
/// We have a bit of code duplication but it avoids a type parameter hell.
impl CompressedInput<VerifyingKey<MNT6_753>, BasePrimeField<MNT6_753>>
    for VerifyingKeyVar<MNT6_753, MNT6PairingVar>
{
    fn new_compressed_input<T: Borrow<VerifyingKey<MNT6_753>>>(
        cs: impl Into<Namespace<BasePrimeField<MNT6_753>>>,
        f: impl FnOnce() -> Result<T, SynthesisError>,
    ) -> Result<Self, SynthesisError> {
        let ns = cs.into();
        let cs = ns.cs();

        f().and_then(|vk| {
            let (bytes, _) = vk
                .borrow()
                .to_field_elements()
                .ok_or(SynthesisError::MalformedVerifyingKey)?;
            let VerifyingKey {
                alpha_g1,
                beta_g2,
                gamma_g2,
                delta_g2,
                gamma_abc_g1,
            } = vk.borrow().clone();

            // Allocate the y bits.
            let bytes_var = UInt8::new_input_vec(ark_relations::ns!(cs, "y_bits"), &bytes)?;
            let bits_var = bytes_var.to_bits_le()?;

            // Then allocate the compressed affines.
            let alpha_g1 = CompressedAffineVar::with_y_bit(
                ark_relations::ns!(cs, "alpha_g1"),
                || Ok(alpha_g1),
                bits_var.get(0).ok_or(SynthesisError::AssignmentMissing)?,
            )?;
            let beta_g2 = CompressedAffineVar::with_y_bit(
                ark_relations::ns!(cs, "beta_g2"),
                || Ok(beta_g2),
                bits_var.get(1).ok_or(SynthesisError::AssignmentMissing)?,
            )?;
            let gamma_g2 = CompressedAffineVar::with_y_bit(
                ark_relations::ns!(cs, "gamma_g2"),
                || Ok(gamma_g2),
                bits_var.get(2).ok_or(SynthesisError::AssignmentMissing)?,
            )?;
            let delta_g2 = CompressedAffineVar::with_y_bit(
                ark_relations::ns!(cs, "delta_g2"),
                || Ok(delta_g2),
                bits_var.get(3).ok_or(SynthesisError::AssignmentMissing)?,
            )?;

            let mut idx = 4;
            let mut gamma_abc_g1_var = vec![];
            for elem in gamma_abc_g1.iter() {
                let elem = CompressedAffineVar::with_y_bit(
                    ark_relations::ns!(cs, "gamma_abc_g1"),
                    || Ok(elem),
                    bits_var.get(idx).ok_or(SynthesisError::AssignmentMissing)?,
                )?;
                gamma_abc_g1_var.push(elem.into_projective());
                idx += 1;
            }

            Ok(Self {
                alpha_g1: alpha_g1.into_projective(),
                beta_g2: beta_g2.into_projective(),
                gamma_g2: gamma_g2.into_projective(),
                delta_g2: delta_g2.into_projective(),
                gamma_abc_g1: gamma_abc_g1_var,
            })
        })
    }
}

/// Allocates a verifying key compressed with `CompressedComposite`.
/// We have a bit of code duplication but it avoids a type parameter hell.
impl CompressedInput<VerifyingKey<MNT4_753>, BasePrimeField<MNT4_753>>
    for VerifyingKeyVar<MNT4_753, MNT4PairingVar>
{
    fn new_compressed_input<T: Borrow<VerifyingKey<MNT4_753>>>(
        cs: impl Into<Namespace<BasePrimeField<MNT4_753>>>,
        f: impl FnOnce() -> Result<T, SynthesisError>,
    ) -> Result<Self, SynthesisError> {
        let ns = cs.into();
        let cs = ns.cs();

        f().and_then(|vk| {
            let (bytes, _) = vk
                .borrow()
                .to_field_elements()
                .ok_or(SynthesisError::MalformedVerifyingKey)?;
            let VerifyingKey {
                alpha_g1,
                beta_g2,
                gamma_g2,
                delta_g2,
                gamma_abc_g1,
            } = vk.borrow().clone();

            // Allocate the y bits.
            let bytes_var = UInt8::new_input_vec(ark_relations::ns!(cs, "y_bits"), &bytes)?;
            let bits_var = bytes_var.to_bits_le()?;

            // Then allocate the compressed affines.
            let alpha_g1 = CompressedAffineVar::with_y_bit(
                ark_relations::ns!(cs, "alpha_g1"),
                || Ok(alpha_g1),
                bits_var.get(0).ok_or(SynthesisError::AssignmentMissing)?,
            )?;
            let beta_g2 = CompressedAffineVar::with_y_bit(
                ark_relations::ns!(cs, "beta_g2"),
                || Ok(beta_g2),
                bits_var.get(1).ok_or(SynthesisError::AssignmentMissing)?,
            )?;
            let gamma_g2 = CompressedAffineVar::with_y_bit(
                ark_relations::ns!(cs, "gamma_g2"),
                || Ok(gamma_g2),
                bits_var.get(2).ok_or(SynthesisError::AssignmentMissing)?,
            )?;
            let delta_g2 = CompressedAffineVar::with_y_bit(
                ark_relations::ns!(cs, "delta_g2"),
                || Ok(delta_g2),
                bits_var.get(3).ok_or(SynthesisError::AssignmentMissing)?,
            )?;

            let mut idx = 4;
            let mut gamma_abc_g1_var = vec![];
            for elem in gamma_abc_g1.iter() {
                let elem = CompressedAffineVar::with_y_bit(
                    ark_relations::ns!(cs, "gamma_abc_g1"),
                    || Ok(elem),
                    bits_var.get(idx).ok_or(SynthesisError::AssignmentMissing)?,
                )?;
                gamma_abc_g1_var.push(elem.into_projective());
                idx += 1;
            }
            Ok(Self {
                alpha_g1: alpha_g1.into_projective(),
                beta_g2: beta_g2.into_projective(),
                gamma_g2: gamma_g2.into_projective(),
                delta_g2: delta_g2.into_projective(),
                gamma_abc_g1: gamma_abc_g1_var,
            })
        })
    }
}
