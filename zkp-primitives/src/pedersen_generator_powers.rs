use std::io::Cursor;

use beserial::{Deserialize, DeserializeWithLength};
use lazy_static::lazy_static;
use nimiq_bls::pedersen::PedersenGenerator;

lazy_static! {
    pub static ref PEDERSEN_GENERATORS: Vec<Vec<PedersenGenerator>> = {
        let serialized =
            include_bytes!(concat!(env!("OUT_DIR"), "/pedersen_generator_powers.data"));
        let mut serialized_cursor = Cursor::new(serialized);

        let count: u16 = Deserialize::deserialize(&mut serialized_cursor).unwrap();
        let mut powers = Vec::with_capacity(count as usize);
        for _ in 0..count {
            powers.push(
                DeserializeWithLength::deserialize::<u16, _>(&mut serialized_cursor).unwrap(),
            );
        }
        powers
    };
}
