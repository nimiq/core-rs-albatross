

use std::time::Instant;

use ark_crypto_primitives::CircuitSpecificSetupSNARK;













use nimiq_nano_sync::NanoZKP;

fn main() {
    println!("====== Parameter generation for Nano Sync initiated ======");
    let start = Instant::now();

    NanoZKP.setup().unwrap();

    println!("====== Parameter generation for Nano Sync finished ======");
    println!("Total time elapsed: {:?} seconds", start.elapsed());
}
