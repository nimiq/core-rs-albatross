use ark_ec::{pairing::Pairing, CurveGroup};
use ark_ff::Field;
use ark_groth16::{constraints::VerifyingKeyVar, VerifyingKey};
use ark_mnt4_753::MNT4_753;
use ark_mnt6_753::MNT6_753;
use ark_r1cs_std::{groups::GroupOpsBounds, pairing::PairingVar, uint8::UInt8};
use ark_relations::r1cs::{ConstraintSystemRef, SynthesisError};
use ark_std::UniformRand;
use log::error;
use nimiq_zkp_primitives::{pedersen::DefaultPedersenParameters95, vk_commitment, vks_commitment};
use rand::Rng;

use super::{
    mnt4::{
        MacroBlockWrapperCircuit, MergerWrapperCircuit, PKTreeNodeCircuit as MNT4PKTreeNodeCircuit,
    },
    mnt6::{
        MacroBlockCircuit, MergerCircuit, PKTreeLeafCircuit,
        PKTreeNodeCircuit as MNT6PKTreeNodeCircuit,
    },
    CircuitInput,
};
use crate::gadgets::{
    pedersen::PedersenParametersVar, serialize::SerializeGadget, vk_commitment::VkCommitmentGadget,
    vks_commitment::VksCommitmentGadget,
};

type BasePrimeField<E> = <<<E as Pairing>::G1 as CurveGroup>::BaseField as Field>::BasePrimeField;

fn dummy_vk<E: Pairing>(num_public_inputs: usize) -> VerifyingKey<E> {
    let mut vk = VerifyingKey::<E>::default();
    for _ in 0..num_public_inputs + 1 {
        vk.gamma_abc_g1.push(Default::default());
    }
    vk
}

#[derive(Debug, Clone)]
pub struct VerifyingKeys {
    merger_wrapper: VerifyingKey<MNT6_753>,
    merger: VerifyingKey<MNT4_753>,
    macro_block_wrapper: VerifyingKey<MNT6_753>,
    macro_block: VerifyingKey<MNT4_753>,
    pk_tree_mnt6: Vec<VerifyingKey<MNT6_753>>,
    pk_tree_mnt4: Vec<VerifyingKey<MNT4_753>>,
}

fn randomize_vk<E: Pairing, R: Rng + ?Sized>(vk: &mut VerifyingKey<E>, rng: &mut R) {
    vk.alpha_g1 = UniformRand::rand(rng);
    vk.beta_g2 = UniformRand::rand(rng);
    vk.gamma_g2 = UniformRand::rand(rng);
    vk.delta_g2 = UniformRand::rand(rng);
    for elem in vk.gamma_abc_g1.iter_mut() {
        *elem = UniformRand::rand(rng);
    }
}

impl UniformRand for VerifyingKeys {
    fn rand<R: Rng + ?Sized>(rng: &mut R) -> Self {
        let mut keys = VerifyingKeys::default();
        randomize_vk(&mut keys.merger_wrapper, rng);
        randomize_vk(&mut keys.merger, rng);
        randomize_vk(&mut keys.macro_block_wrapper, rng);
        randomize_vk(&mut keys.macro_block, rng);
        for key in keys.pk_tree_mnt6.iter_mut() {
            randomize_vk(key, rng);
        }
        for key in keys.pk_tree_mnt4.iter_mut() {
            randomize_vk(key, rng);
        }
        keys
    }
}

impl Default for VerifyingKeys {
    fn default() -> Self {
        let mut keys = Self {
            merger_wrapper: dummy_vk(MergerWrapperCircuit::NUM_INPUTS),
            merger: dummy_vk(MergerCircuit::NUM_INPUTS),
            macro_block_wrapper: dummy_vk(MacroBlockWrapperCircuit::NUM_INPUTS),
            macro_block: dummy_vk(MacroBlockCircuit::NUM_INPUTS),
            pk_tree_mnt6: vec![],
            pk_tree_mnt4: vec![],
        };
        for tree_level in 0..=5 {
            if tree_level % 2 == MNT6_753::VK_COMMITMENT_INDEX {
                keys.pk_tree_mnt6
                    .push(dummy_vk(MNT4PKTreeNodeCircuit::num_inputs(tree_level)));
            } else {
                // The leaf node does not require the vks commitment.
                let num_inputs = if tree_level == 5 {
                    PKTreeLeafCircuit::num_inputs(tree_level)
                } else {
                    MNT6PKTreeNodeCircuit::num_inputs(tree_level)
                };
                keys.pk_tree_mnt4.push(dummy_vk(num_inputs));
            }
        }
        keys
    }
}

impl VerifyingKeys {
    pub fn new(
        merger_wrapper: VerifyingKey<MNT6_753>,
        merger: VerifyingKey<MNT4_753>,
        macro_block_wrapper: VerifyingKey<MNT6_753>,
        macro_block: VerifyingKey<MNT4_753>,
        pk_tree0: VerifyingKey<MNT6_753>,
        pk_tree1: VerifyingKey<MNT4_753>,
        pk_tree2: VerifyingKey<MNT6_753>,
        pk_tree3: VerifyingKey<MNT4_753>,
        pk_tree4: VerifyingKey<MNT6_753>,
        pk_tree_leaf: VerifyingKey<MNT4_753>,
    ) -> Self {
        Self {
            merger_wrapper,
            merger,
            macro_block_wrapper,
            macro_block,
            pk_tree_mnt6: vec![pk_tree0, pk_tree2, pk_tree4],
            pk_tree_mnt4: vec![pk_tree1, pk_tree3, pk_tree_leaf],
        }
    }

    /// We commit to mnt4 and mnt6 vks separately and concatenate the commitments.
    /// We first commit to the individual keys and then hash the commitments together.
    /// This way we can unpack the respective commitment on the right curve.
    /// Passing them as one saves us one public input.
    pub fn commitment(&self) -> [u8; 95 * 2] {
        let mnt6_commitments = PairingRelatedKeys::<MNT6_753>::get_keys(self)
            .iter()
            .map(|key| vk_commitment(key))
            .collect::<Vec<_>>();
        let mnt6_commitment = vks_commitment::<MNT6_753>(&mnt6_commitments);

        let mnt4_commitments = PairingRelatedKeys::<MNT4_753>::get_keys(self)
            .iter()
            .map(|key| vk_commitment(key))
            .collect::<Vec<_>>();
        let mnt4_commitment = vks_commitment::<MNT4_753>(&mnt4_commitments);
        let mut final_commitment = [0u8; 95 * 2];
        final_commitment[..95].copy_from_slice(&mnt6_commitment);
        final_commitment[95..].copy_from_slice(&mnt4_commitment);
        final_commitment
    }
}

pub trait PairingRelatedKeys<E: Pairing> {
    fn get_keys(&self) -> Vec<&VerifyingKey<E>>;
    fn get_key(&self, circuit_id: CircuitId) -> Option<&VerifyingKey<E>>;
}
impl PairingRelatedKeys<MNT6_753> for VerifyingKeys {
    fn get_keys(&self) -> Vec<&VerifyingKey<MNT6_753>> {
        let mut mnt6_keys = vec![&self.merger_wrapper, &self.macro_block_wrapper];
        for pk_tree_vk in self.pk_tree_mnt6.iter() {
            mnt6_keys.push(pk_tree_vk);
        }
        mnt6_keys
    }

    fn get_key(&self, circuit_id: CircuitId) -> Option<&VerifyingKey<MNT6_753>> {
        match circuit_id {
            CircuitId::MergerWrapper => Some(&self.merger_wrapper),
            CircuitId::MacroBlockWrapper => Some(&self.macro_block_wrapper),
            CircuitId::PkTree(i) if i % 2 == MNT6_753::VK_COMMITMENT_INDEX => {
                self.pk_tree_mnt6.get(i / 2)
            }
            _ => None,
        }
    }
}
impl PairingRelatedKeys<MNT4_753> for VerifyingKeys {
    fn get_keys(&self) -> Vec<&VerifyingKey<MNT4_753>> {
        let mut mnt4_keys = vec![&self.merger, &self.macro_block];
        for pk_tree_vk in self.pk_tree_mnt4.iter() {
            mnt4_keys.push(pk_tree_vk);
        }
        mnt4_keys
    }

    fn get_key(&self, circuit_id: CircuitId) -> Option<&VerifyingKey<MNT4_753>> {
        match circuit_id {
            CircuitId::Merger => Some(&self.merger),
            CircuitId::MacroBlock => Some(&self.macro_block),
            CircuitId::PkTree(i) if i % 2 == MNT4_753::VK_COMMITMENT_INDEX => {
                self.pk_tree_mnt4.get(i / 2)
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum CircuitId {
    MergerWrapper,
    Merger,
    MacroBlockWrapper,
    MacroBlock,
    PkTree(usize),
}

impl CircuitId {
    /// Returns the index tuple for the circuit key.
    /// The first position depends on the curve type
    /// and determines the commitment the key is part of:
    /// 0 = MNT6_753
    /// 1 = MNT4_753
    /// The second index is the index inside the commitment.
    fn index(&self) -> (usize, usize) {
        match self {
            CircuitId::MergerWrapper => (0, 0),
            CircuitId::Merger => (1, 0),
            CircuitId::MacroBlockWrapper => (0, 1),
            CircuitId::MacroBlock => (1, 1),
            CircuitId::PkTree(i) => (i % 2, 2 + (i / 2)),
        }
    }
}

/// A helper trait that makes sure we cannot access keys on the wrong curve.
pub trait VkCommitmentIndex {
    const VK_COMMITMENT_INDEX: usize;
}
impl VkCommitmentIndex for MNT6_753 {
    const VK_COMMITMENT_INDEX: usize = 0;
}
impl VkCommitmentIndex for MNT4_753 {
    const VK_COMMITMENT_INDEX: usize = 1;
}

pub struct VerifyingKeyHelper<P: Pairing + VkCommitmentIndex + DefaultPedersenParameters95> {
    keys: VerifyingKeys,
    vks_commitment_gadget: VksCommitmentGadget<P>,
}

impl<P: Pairing + VkCommitmentIndex + DefaultPedersenParameters95> VerifyingKeyHelper<P>
where
    VerifyingKeys: PairingRelatedKeys<P>,
{
    pub fn new_and_verify<PV: PairingVar<P, BasePrimeField<P>>>(
        cs: ConstraintSystemRef<BasePrimeField<P>>,
        keys: VerifyingKeys,
        commitment: &[UInt8<BasePrimeField<P>>],
        pedersen_generators: &PedersenParametersVar<P::G1, PV::G1Var>,
    ) -> Result<Self, SynthesisError>
    where
        PV::G1Var: SerializeGadget<BasePrimeField<P>>,
        for<'a> &'a PV::G1Var: GroupOpsBounds<'a, P::G1, PV::G1Var>,
    {
        let sub_commitment =
            &commitment[P::VK_COMMITMENT_INDEX * 95..(P::VK_COMMITMENT_INDEX + 1) * 95];
        let vk_commitments = PairingRelatedKeys::<P>::get_keys(&keys)
            .iter()
            .map(|key| Some(vk_commitment(key)))
            .collect::<Vec<_>>();

        Ok(Self {
            keys,
            vks_commitment_gadget: VksCommitmentGadget::new_and_verify::<PV>(
                cs.clone(),
                vk_commitments,
                sub_commitment.to_vec(),
                pedersen_generators,
            )?,
        })
    }

    pub fn get_and_verify_vk<PV: PairingVar<P, BasePrimeField<P>>>(
        &self,
        cs: ConstraintSystemRef<BasePrimeField<P>>,
        circuit_id: CircuitId,
        pedersen_generators: &PedersenParametersVar<P::G1, PV::G1Var>,
    ) -> Result<VerifyingKeyVar<P, PV>, SynthesisError>
    where
        for<'a> &'a PV::G1Var: GroupOpsBounds<'a, P::G1, PV::G1Var>,
        PV::G1Var: SerializeGadget<BasePrimeField<P>>,
        PV::G2Var: SerializeGadget<BasePrimeField<P>>,
    {
        let vk = PairingRelatedKeys::<P>::get_key(&self.keys, circuit_id).ok_or_else(|| {
            error!("Could not load key {:?}.", circuit_id);
            SynthesisError::Unsatisfiable
        })?;
        let (c_index, i) = circuit_id.index();
        assert_eq!(c_index, P::VK_COMMITMENT_INDEX);
        let commitment = self.vks_commitment_gadget.vk_commitments[i].clone();
        let vk_commitment_gadget = VkCommitmentGadget::<P, PV>::new_and_verify(
            cs.clone(),
            vk,
            commitment,
            pedersen_generators,
        )?;
        Ok(vk_commitment_gadget.vk)
    }
}

#[cfg(test)]
mod tests {
    use ark_mnt4_753::{constraints::PairingVar as MNT4PairingVar, Fq as MNT4Fq, MNT4_753};
    use ark_mnt6_753::{constraints::PairingVar as MNT6PairingVar, Fq as MNT6Fq, MNT6_753};
    use ark_r1cs_std::{alloc::AllocVar, R1CSVar};
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::{test_rng, UniformRand};
    use nimiq_zkp_primitives::{pedersen::pedersen_parameters_mnt4, pedersen_parameters_mnt6};

    use super::*;
    use crate::gadgets::{
        mnt4::DefaultPedersenParametersVar as MNT4PedersenParametersVar,
        mnt6::DefaultPedersenParametersVar as MNT6PedersenParametersVar,
        vk_commitment::VkCommitmentWindow,
    };

    fn assert_eq_vk<E: Pairing, P: PairingVar<E, BasePrimeField<E>>>(
        vk: &VerifyingKey<E>,
        vk_var: &VerifyingKeyVar<E, P>,
    ) {
        assert_eq!(vk.alpha_g1, vk_var.alpha_g1.value().unwrap().into());
        assert_eq!(vk.beta_g2, vk_var.beta_g2.value().unwrap().into());
        assert_eq!(vk.gamma_g2, vk_var.gamma_g2.value().unwrap().into());
        assert_eq!(vk.delta_g2, vk_var.delta_g2.value().unwrap().into());
        for (a, b) in vk.gamma_abc_g1.iter().zip(vk_var.gamma_abc_g1.iter()) {
            assert_eq!(a, &b.value().unwrap().into());
        }
    }

    #[test]
    fn vks_mnt6() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT6Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create verifying keys.
        let keys = VerifyingKeys::rand(rng);

        let main_commitment = keys.commitment();
        let commitment_var = UInt8::<MNT6Fq>::new_input_vec(cs.clone(), &main_commitment).unwrap();

        // Evaluate vk commitment using the gadget version.
        let pedersen_generators = MNT6PedersenParametersVar::new_constant(
            cs.clone(),
            pedersen_parameters_mnt6().sub_window::<VkCommitmentWindow>(),
        )
        .unwrap();
        let vks_gadget = VerifyingKeyHelper::<MNT6_753>::new_and_verify::<MNT6PairingVar>(
            cs.clone(),
            keys.clone(),
            &commitment_var,
            &pedersen_generators,
        )
        .unwrap();

        let vk_var = vks_gadget
            .get_and_verify_vk::<MNT6PairingVar>(
                cs.clone(),
                CircuitId::MergerWrapper,
                &pedersen_generators,
            )
            .unwrap();
        assert_eq_vk(&keys.merger_wrapper, &vk_var);

        let vk_var = vks_gadget
            .get_and_verify_vk::<MNT6PairingVar>(
                cs.clone(),
                CircuitId::MacroBlockWrapper,
                &pedersen_generators,
            )
            .unwrap();
        assert_eq_vk(&keys.macro_block_wrapper, &vk_var);

        for i in [0, 2, 4] {
            let vk_var = vks_gadget
                .get_and_verify_vk::<MNT6PairingVar>(
                    cs.clone(),
                    CircuitId::PkTree(i),
                    &pedersen_generators,
                )
                .unwrap();
            assert_eq_vk(&keys.pk_tree_mnt6[i / 2], &vk_var);
        }

        assert!(cs.is_satisfied().unwrap());

        println!("Num constraints: {}", cs.num_constraints());
    }

    #[test]
    fn vks_mnt4() {
        // Initialize the constraint system.
        let cs = ConstraintSystem::<MNT4Fq>::new_ref();

        // Create random number generator.
        let rng = &mut test_rng();

        // Create verifying keys.
        let keys = VerifyingKeys::rand(rng);

        let main_commitment = keys.commitment();
        let commitment_var = UInt8::<MNT4Fq>::new_input_vec(cs.clone(), &main_commitment).unwrap();

        // Evaluate vk commitment using the gadget version.
        let pedersen_generators = MNT4PedersenParametersVar::new_constant(
            cs.clone(),
            pedersen_parameters_mnt4().sub_window::<VkCommitmentWindow>(),
        )
        .unwrap();
        let vks_gadget = VerifyingKeyHelper::<MNT4_753>::new_and_verify::<MNT4PairingVar>(
            cs.clone(),
            keys.clone(),
            &commitment_var,
            &pedersen_generators,
        )
        .unwrap();

        let vk_var = vks_gadget
            .get_and_verify_vk::<MNT4PairingVar>(
                cs.clone(),
                CircuitId::Merger,
                &pedersen_generators,
            )
            .unwrap();
        assert_eq_vk(&keys.merger, &vk_var);

        let vk_var = vks_gadget
            .get_and_verify_vk::<MNT4PairingVar>(
                cs.clone(),
                CircuitId::MacroBlock,
                &pedersen_generators,
            )
            .unwrap();
        assert_eq_vk(&keys.macro_block, &vk_var);

        for i in [1, 3, 5] {
            let vk_var = vks_gadget
                .get_and_verify_vk::<MNT4PairingVar>(
                    cs.clone(),
                    CircuitId::PkTree(i),
                    &pedersen_generators,
                )
                .unwrap();
            assert_eq_vk(&keys.pk_tree_mnt4[i / 2], &vk_var);
        }

        assert!(cs.is_satisfied().unwrap());

        println!("Num constraints: {}", cs.num_constraints());
    }
}
