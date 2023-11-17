use std::{fs::File, path::Path};

use ark_ec::{pairing::Pairing, scalar_mul::fixed_base::FixedBase, CurveGroup, Group};
use ark_ff::{Field, PrimeField, UniformRand, Zero};
use ark_groth16::{
    r1cs_to_qap::{LibsnarkReduction, R1CSToQAP},
    Proof, ProvingKey, VerifyingKey,
};
use ark_mnt6_753::MNT6_753;
use ark_poly::{EvaluationDomain, GeneralEvaluationDomain};
use ark_relations::r1cs::{
    ConstraintSynthesizer, ConstraintSystem, OptimizationGoal, Result as R1CSResult,
    SynthesisError, SynthesisMode,
};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use ark_std::{cfg_into_iter, cfg_iter, rand::Rng};
use nimiq_zkp_primitives::NanoZKPError;
use rand::CryptoRng;
#[cfg(feature = "parallel")]
use rayon::iter::{
    IndexedParallelIterator, IntoParallelIterator, IntoParallelRefIterator, ParallelIterator,
};

use crate::{circuits::mnt4::MergerWrapperCircuit, setup::keys_to_file};

/// Seed to be used to create toxic waste for unit tests.
pub const UNIT_TOXIC_WASTE_SEED: [u8; 32] = [
    1, 0, 52, 0, 0, 0, 0, 0, 1, 0, 10, 0, 22, 32, 0, 0, 2, 0, 55, 49, 0, 11, 0, 0, 3, 0, 0, 0, 0,
    0, 2, 92,
];

#[derive(CanonicalSerialize, CanonicalDeserialize)]
pub struct ToxicWaste<E: Pairing> {
    alpha: E::ScalarField,
    beta: E::ScalarField,
    gamma: E::ScalarField,
    delta: E::ScalarField,

    g1_generator: E::G1,
    g2_generator: E::G2,

    abc: Vec<E::ScalarField>,
}

impl<E: Pairing> UniformRand for ToxicWaste<E> {
    fn rand<R: Rng + ?Sized>(rng: &mut R) -> Self {
        Self {
            alpha: E::ScalarField::rand(rng),
            beta: E::ScalarField::rand(rng),
            gamma: E::ScalarField::rand(rng),
            delta: E::ScalarField::rand(rng),
            g1_generator: E::G1::rand(rng),
            g2_generator: E::G2::rand(rng),
            abc: vec![],
        }
    }
}

impl<E: Pairing> ToxicWaste<E> {
    pub fn setup_groth16<C>(circuit: C, rng: &mut impl Rng) -> R1CSResult<(Self, ProvingKey<E>)>
    where
        C: ConstraintSynthesizer<E::ScalarField>,
    {
        let mut toxic_waste = ToxicWaste::rand(rng);
        let proving_key =
            toxic_waste.generate_parameters_with_qap::<_, LibsnarkReduction>(circuit, rng)?;
        Ok((toxic_waste, proving_key))
    }

    /// Create parameters for a circuit, given some toxic waste, R1CS to QAP calculator and group generators
    fn generate_parameters_with_qap<C, QAP: R1CSToQAP>(
        &mut self,
        circuit: C,
        rng: &mut impl Rng,
    ) -> R1CSResult<ProvingKey<E>>
    where
        C: ConstraintSynthesizer<E::ScalarField>,
    {
        type D<F> = GeneralEvaluationDomain<F>;

        let cs = ConstraintSystem::new_ref();
        cs.set_optimization_goal(OptimizationGoal::Constraints);
        cs.set_mode(SynthesisMode::Setup);

        // Synthesize the circuit.
        circuit.generate_constraints(cs.clone())?;

        cs.finalize();

        // Following is the mapping of symbols from the Groth16 paper to this implementation
        // l -> num_instance_variables
        // m -> qap_num_variables
        // x -> t
        // t(x) - zt
        // u_i(x) -> a
        // v_i(x) -> b
        // w_i(x) -> c

        ///////////////////////////////////////////////////////////////////////////
        let domain_size = cs.num_constraints() + cs.num_instance_variables();
        let domain = D::new(domain_size).ok_or(SynthesisError::PolynomialDegreeTooLarge)?;
        let t = domain.sample_element_outside_domain(rng);
        ///////////////////////////////////////////////////////////////////////////

        let num_instance_variables = cs.num_instance_variables();
        let (a, b, c, zt, qap_num_variables, m_raw) =
            QAP::instance_map_with_evaluation::<E::ScalarField, D<E::ScalarField>>(cs, &t)?;

        // Compute query densities
        let non_zero_a: usize = cfg_into_iter!(0..qap_num_variables)
            .map(|i| usize::from(!a[i].is_zero()))
            .sum();

        let non_zero_b: usize = cfg_into_iter!(0..qap_num_variables)
            .map(|i| usize::from(!b[i].is_zero()))
            .sum();

        let scalar_bits = E::ScalarField::MODULUS_BIT_SIZE as usize;

        let gamma_inverse = self
            .gamma
            .inverse()
            .ok_or(SynthesisError::UnexpectedIdentity)?;
        let delta_inverse = self
            .delta
            .inverse()
            .ok_or(SynthesisError::UnexpectedIdentity)?;

        let gamma_abc = cfg_iter!(a[..num_instance_variables])
            .zip(&b[..num_instance_variables])
            .zip(&c[..num_instance_variables])
            .map(|((a, b), c)| (self.beta * a + self.alpha * b + c) * gamma_inverse)
            .collect::<Vec<_>>();

        self.abc = cfg_iter!(a[..num_instance_variables])
            .zip(&b[..num_instance_variables])
            .zip(&c[..num_instance_variables])
            .map(|((a, b), c)| self.beta * a + self.alpha * b + c)
            .collect::<Vec<_>>();

        let l = cfg_iter!(a[num_instance_variables..])
            .zip(&b[num_instance_variables..])
            .zip(&c[num_instance_variables..])
            .map(|((a, b), c)| (self.beta * a + self.alpha * b + c) * delta_inverse)
            .collect::<Vec<_>>();

        drop(c);

        // Compute B window table
        let g2_window = FixedBase::get_mul_window_size(non_zero_b);
        let g2_table =
            FixedBase::get_window_table::<E::G2>(scalar_bits, g2_window, self.g2_generator);

        // Compute the B-query in G2
        let b_g2_query = FixedBase::msm::<E::G2>(scalar_bits, g2_window, &g2_table, &b);
        drop(g2_table);

        // Compute G window table
        let g1_window =
            FixedBase::get_mul_window_size(non_zero_a + non_zero_b + qap_num_variables + m_raw + 1);
        let g1_table =
            FixedBase::get_window_table::<E::G1>(scalar_bits, g1_window, self.g1_generator);

        // Generate the R1CS proving key
        let alpha_g1 = self.g1_generator.mul_bigint(&self.alpha.into_bigint());
        let beta_g1 = self.g1_generator.mul_bigint(&self.beta.into_bigint());
        let beta_g2 = self.g2_generator.mul_bigint(&self.beta.into_bigint());
        let delta_g1 = self.g1_generator.mul_bigint(&self.delta.into_bigint());
        let delta_g2 = self.g2_generator.mul_bigint(&self.delta.into_bigint());

        // Compute the A-query
        let a_query = FixedBase::msm::<E::G1>(scalar_bits, g1_window, &g1_table, &a);
        drop(a);

        // Compute the B-query in G1
        let b_g1_query = FixedBase::msm::<E::G1>(scalar_bits, g1_window, &g1_table, &b);
        drop(b);

        // Compute the H-query
        let h_query = FixedBase::msm::<E::G1>(
            scalar_bits,
            g1_window,
            &g1_table,
            &QAP::h_query_scalars::<_, D<E::ScalarField>>(m_raw - 1, t, zt, delta_inverse)?,
        );

        // Compute the L-query
        let l_query = FixedBase::msm::<E::G1>(scalar_bits, g1_window, &g1_table, &l);
        drop(l);

        // Generate R1CS verification key
        let gamma_g2 = self.g2_generator.mul_bigint(&self.gamma.into_bigint());
        let gamma_abc_g1 = FixedBase::msm::<E::G1>(scalar_bits, g1_window, &g1_table, &gamma_abc);

        drop(g1_table);

        let vk = VerifyingKey::<E> {
            alpha_g1: alpha_g1.into_affine(),
            beta_g2: beta_g2.into_affine(),
            gamma_g2: gamma_g2.into_affine(),
            delta_g2: delta_g2.into_affine(),
            gamma_abc_g1: E::G1::normalize_batch(&gamma_abc_g1),
        };

        let a_query = E::G1::normalize_batch(&a_query);
        let b_g1_query = E::G1::normalize_batch(&b_g1_query);
        let b_g2_query = E::G2::normalize_batch(&b_g2_query);
        let h_query = E::G1::normalize_batch(&h_query);
        let l_query = E::G1::normalize_batch(&l_query);

        Ok(ProvingKey {
            vk,
            beta_g1: beta_g1.into_affine(),
            delta_g1: delta_g1.into_affine(),
            a_query,
            b_g1_query,
            b_g2_query,
            h_query,
            l_query,
        })
    }

    /// Implements the simulator as given in the [Groth16 paper](https://eprint.iacr.org/2016/260.pdf)
    /// from the toxic waste.
    pub fn simulate_proof(&self, input: &[E::ScalarField], rng: &mut impl Rng) -> Proof<E> {
        let a = E::ScalarField::rand(rng);
        let b = E::ScalarField::rand(rng);

        let ab = a * b;
        let alpha_beta = self.alpha * self.beta;

        assert_eq!(input.len() + 1, self.abc.len());
        let mut processed_input = self.abc[0];
        for (i, b) in input.iter().zip(self.abc.iter().skip(1)) {
            processed_input += *i * *b;
        }
        let delta_inv = self.delta.inverse().unwrap();
        let c = (ab - alpha_beta - processed_input) * delta_inv;

        Proof {
            a: self.g1_generator.mul_bigint(&a.into_bigint()).into_affine(),
            b: self.g2_generator.mul_bigint(&b.into_bigint()).into_affine(),
            c: self.g1_generator.mul_bigint(&c.into_bigint()).into_affine(),
        }
    }
}

pub fn setup_merger_wrapper_simulation<R: Rng + CryptoRng>(
    rng: &mut R,
    path: &Path,
) -> Result<ToxicWaste<MNT6_753>, NanoZKPError> {
    // Create parameters for our circuit
    let circuit = MergerWrapperCircuit::rand(rng);

    let (toxic_waste, pk) = ToxicWaste::setup_groth16(circuit, rng)?;

    // Save keys to file.
    keys_to_file(&pk, &pk.vk, "merger_wrapper", path)?;

    // Save toxic waste to file.
    let mut file = File::create(path.join("toxic_waste.bin"))?;
    toxic_waste.serialize_uncompressed(&mut file)?;
    file.sync_all()?;

    Ok(toxic_waste)
}

#[cfg(test)]
mod tests {
    use ark_crypto_primitives::snark::SNARK;
    use ark_ff::ToConstraintField;
    use ark_groth16::Groth16;
    use ark_mnt4_753::Fq as FqMNT4;
    use ark_r1cs_std::{prelude::EqGadget, uint8::UInt8};
    use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
    use ark_std::test_rng;
    use nimiq_test_log::test;

    use super::*;

    #[derive(Clone)]
    pub struct InnerCircuit {
        // Witnesses (private)
        red_priv: Vec<u8>,
        blue_priv: Vec<u8>,
        // Inputs (public)
        red_pub: Vec<u8>,
        blue_pub: Vec<u8>,
    }

    impl ConstraintSynthesizer<FqMNT4> for InnerCircuit {
        /// This function generates the constraints for the circuit.
        fn generate_constraints(
            self,
            cs: ConstraintSystemRef<FqMNT4>,
        ) -> Result<(), SynthesisError> {
            // Allocate all the witnesses.
            let red_priv_var = UInt8::<FqMNT4>::new_witness_vec(cs.clone(), &self.red_priv[..])?;

            let blue_priv_var = UInt8::<FqMNT4>::new_witness_vec(cs.clone(), &self.blue_priv[..])?;

            // Allocate all the inputs.
            let red_pub_var = UInt8::<FqMNT4>::new_input_vec(cs.clone(), &self.red_pub[..])?;

            let blue_pub_var = UInt8::<FqMNT4>::new_input_vec(cs, &self.blue_pub[..])?;

            // Compare the bytes.
            red_priv_var.enforce_equal(&red_pub_var)?;

            blue_priv_var.enforce_equal(&blue_pub_var)?;

            Ok(())
        }
    }

    #[test]
    fn test_proof_simulation() {
        let mut rng = test_rng();

        let circuit = InnerCircuit {
            red_priv: vec![125, 148],
            blue_priv: vec![38, 127],
            red_pub: vec![73, 91],
            blue_pub: vec![18, 93],
        };

        let (toxic_waste, pk) = ToxicWaste::setup_groth16(circuit.clone(), &mut rng).unwrap();

        let mut input = vec![];
        input.extend_from_slice(&circuit.red_pub.to_field_elements().unwrap());
        input.extend_from_slice(&circuit.blue_pub.to_field_elements().unwrap());

        let proof: Proof<MNT6_753> = toxic_waste.simulate_proof(&input, &mut rng);

        assert!(Groth16::<_, LibsnarkReduction>::verify(&pk.vk, &input, &proof).unwrap());
    }
}
