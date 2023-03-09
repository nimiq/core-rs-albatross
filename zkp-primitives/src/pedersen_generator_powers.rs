use std::io::Cursor;

use ark_mnt6_753::G1Projective;
use lazy_static::lazy_static;

use beserial::Deserialize;
use nimiq_pedersen_generators::generators::PedersenParameters;

lazy_static! {
    pub static ref PEDERSEN_PARAMETERS: PedersenParameters<G1Projective> = {
        let serialized =
            include_bytes!(concat!(env!("OUT_DIR"), "/pedersen_generator_powers.data"));
        let mut serialized_cursor = Cursor::new(serialized);

        Deserialize::deserialize(&mut serialized_cursor).unwrap()
    };
}
