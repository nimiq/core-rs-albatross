use std::{fs::File, path::Path, time::Instant};

use ark_ec::pairing::Pairing;
use ark_ff::Field;
use ark_groth16::VerifyingKey;
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystem, OptimizationGoal};
use ark_serialize::CanonicalDeserialize;
use nimiq_zkp_circuits::{
    circuits::{mnt4, mnt6},
    setup::all_files_created,
    DEFAULT_KEYS_PATH,
};
use rand::{thread_rng, Rng};

fn evaluate_circuit<F: Field, C: ConstraintSynthesizer<F> + Clone>(circuit: C, circuit_name: &str) {
    let cs = ConstraintSystem::new_ref();
    cs.set_optimization_goal(OptimizationGoal::Constraints);
    circuit.clone().generate_constraints(cs.clone()).unwrap();
    cs.finalize();
    let num_optimized_constraints = cs.num_constraints();

    let cs = ConstraintSystem::new_ref();
    cs.set_optimization_goal(OptimizationGoal::Weight);
    circuit.generate_constraints(cs.clone()).unwrap();
    cs.finalize();
    let num_optimized_weight = cs.num_constraints();

    println!(
        "- {}: opt_constraints={}, opt_weight={}",
        circuit_name, num_optimized_constraints, num_optimized_weight
    );
}

fn load_vk<P: Pairing>(vk_file: &str) -> VerifyingKey<P> {
    let mut file = File::open(
        Path::new(DEFAULT_KEYS_PATH)
            .join("verifying_keys")
            .join(format!("{vk_file}.bin")),
    )
    .unwrap();

    VerifyingKey::deserialize_uncompressed_unchecked(&mut file).unwrap()
}

fn main() {
    if !all_files_created(Path::new(DEFAULT_KEYS_PATH), false) {
        eprintln!(
            "To run this binary, there need to be verifying keys in the `{}` folder.",
            DEFAULT_KEYS_PATH
        );
        return;
    }

    // Print sizes for each circuit.
    println!("====== ZKP constraint estimation initiated ======");
    let start = Instant::now();
    let mut rng = thread_rng();

    let circuit: mnt6::PKTreeLeafCircuit = rng.gen();
    evaluate_circuit(circuit, "pk_tree_leaf");

    let vk = load_vk("pk_tree_5");
    let circuit = mnt4::PKTreeNodeCircuit::rand(1, vk, &mut rng);
    evaluate_circuit(circuit, "pk_tree_node mnt4");

    let vk = load_vk("pk_tree_4");
    let circuit = mnt6::PKTreeNodeCircuit::rand(1, vk, &mut rng);
    evaluate_circuit(circuit, "pk_tree_node mnt6");

    let vk = load_vk("pk_tree_0");
    let circuit = mnt6::MacroBlockCircuit::rand(vk, &mut rng);
    evaluate_circuit(circuit, "macro_block");

    let vk = load_vk("macro_block");
    let circuit = mnt4::MacroBlockWrapperCircuit::rand(vk, &mut rng);
    evaluate_circuit(circuit, "macro_block_wrapper");

    let vk = load_vk("macro_block_wrapper");
    let circuit = mnt6::MergerCircuit::rand(vk, &mut rng);
    evaluate_circuit(circuit, "merger");

    let vk = load_vk("merger");
    let circuit = mnt4::MergerWrapperCircuit::rand(vk, &mut rng);
    evaluate_circuit(circuit, "merger_wrapper");

    println!("====== ZKP constraint estimation finished ======");
    println!("Total time elapsed: {:?} seconds", start.elapsed());
}
