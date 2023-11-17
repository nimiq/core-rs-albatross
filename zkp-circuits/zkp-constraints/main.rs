use std::{io, time::Instant};

use ark_ff::Field;
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystem, OptimizationGoal};
use log::{info, level_filters::LevelFilter};
use nimiq_log::TargetsExt;
use nimiq_zkp_circuits::circuits::{mnt4, mnt6};
use rand::{thread_rng, Rng};
use tracing_subscriber::{filter::Targets, layer::SubscriberExt, util::SubscriberInitExt};

fn evaluate_circuit<F: Field, C: ConstraintSynthesizer<F> + Clone>(circuit: C, circuit_name: &str) {
    let cs = ConstraintSystem::new_ref();
    cs.set_optimization_goal(OptimizationGoal::Constraints);
    circuit.clone().generate_constraints(cs.clone()).unwrap();
    cs.finalize();
    let num_optimized_constraints = cs.num_constraints().next_power_of_two().ilog2();

    let cs = ConstraintSystem::new_ref();
    cs.set_optimization_goal(OptimizationGoal::Weight);
    circuit.generate_constraints(cs.clone()).unwrap();
    cs.finalize();
    let num_optimized_weight = cs.num_constraints().next_power_of_two().ilog2();

    info!(
        "- {}: opt_constraints=2^{}, opt_weight=2^{}",
        circuit_name, num_optimized_constraints, num_optimized_weight
    );
}

fn main() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(io::stderr))
        .with(
            Targets::new()
                .with_default(LevelFilter::INFO)
                .with_nimiq_targets(LevelFilter::DEBUG)
                .with_target("r1cs", LevelFilter::WARN)
                .with_env(),
        )
        .init();

    // Print sizes for each circuit.
    info!("====== ZKP constraint estimation initiated ======");
    let start = Instant::now();
    let mut rng = thread_rng();

    let circuit: mnt6::PKTreeLeafCircuit = rng.gen();
    evaluate_circuit(circuit, "pk_tree_leaf");

    let circuit = mnt4::PKTreeNodeCircuit::rand(0, &mut rng);
    evaluate_circuit(circuit, "pk_tree_node mnt4");

    let circuit = mnt6::PKTreeNodeCircuit::rand(1, &mut rng);
    evaluate_circuit(circuit, "pk_tree_node mnt6");

    let circuit = mnt6::MacroBlockCircuit::rand(&mut rng);
    evaluate_circuit(circuit, "macro_block");

    let circuit = mnt4::MacroBlockWrapperCircuit::rand(&mut rng);
    evaluate_circuit(circuit, "macro_block_wrapper");

    let circuit = mnt6::MergerCircuit::rand(&mut rng);
    evaluate_circuit(circuit, "merger");

    let circuit = mnt4::MergerWrapperCircuit::rand(&mut rng);
    evaluate_circuit(circuit, "merger_wrapper");

    info!("====== ZKP constraint estimation finished ======");
    info!("Total time elapsed: {:?} seconds", start.elapsed());
}
