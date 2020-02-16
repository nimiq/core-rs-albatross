use algebra::bytes::FromBytes;
use algebra::curves::bls12_377::{G1Affine, G1Projective, G2Affine, G2Projective};
use algebra::fields::bls12_377::{Fq, Fq2};
use algebra::{AffineCurve, BigInteger384, Group, PrimeField};
use crypto_primitives::crh::pedersen::{PedersenCRH, PedersenParameters, PedersenWindow};
use lazy_static::lazy_static;

pub type CRH = PedersenCRH<G1Projective, CRHWindow>;

// Generated via https://github.com/nimiq/generator-generation
// CRH generators
const G1_X_1: &str = "00B9EACABE8A2BD0BDDF7ED1DC14BAEAAAEDC63EF62ECECFACFF0C2A59D84DC1B6A64E36BC7E9FE29AF6CA3A87AA398F";
const G1_Y_1: &str = "0181E06254E9432CA7C3FD61CEA96175D6D99BB78A1FE848F354D7FB727A50A7F3EFBEF595609BA9B3C03EA3FB3DFC79";
const G1_X_2: &str = "00B9EACABE8A2BD0BDDF7ED1DC14BAEAAAEDC63EF62ECECFACFF0C2A59D84DC1B6A64E36BC7E9FE29AF6CA3A87AA398F";
const G1_Y_2: &str = "002C59E3C2DBCDBE1E77085E9DF7E7C543493E3B76D52B462B9E8A34478EF758231B9E4E9A9F6456D148815C04C20388";
const G1_X_3: &str = "00B9EACABE8A2BD0BDDF7ED1DC14BAEAAAEDC63EF62ECECFACFF0C2A59D84DC1B6A64E36BC7E9FE29AF6CA3A87AA398F";
const G1_Y_3: &str = "002C59E3C2DBCDBE1E77085E9DF7E7C543493E3B76D52B462B9E8A34478EF758231B9E4E9A9F6456D148815C04C20388";
const G1_X_4: &str = "00B9EACABE8A2BD0BDDF7ED1DC14BAEAAAEDC63EF62ECECFACFF0C2A59D84DC1B6A64E36BC7E9FE29AF6CA3A87AA398F";
const G1_Y_4: &str = "002C59E3C2DBCDBE1E77085E9DF7E7C543493E3B76D52B462B9E8A34478EF758231B9E4E9A9F6456D148815C04C20388";
const G1_X_5: &str = "00B9EACABE8A2BD0BDDF7ED1DC14BAEAAAEDC63EF62ECECFACFF0C2A59D84DC1B6A64E36BC7E9FE29AF6CA3A87AA398F";
const G1_Y_5: &str = "002C59E3C2DBCDBE1E77085E9DF7E7C543493E3B76D52B462B9E8A34478EF758231B9E4E9A9F6456D148815C04C20388";
const G1_X_6: &str = "00B9EACABE8A2BD0BDDF7ED1DC14BAEAAAEDC63EF62ECECFACFF0C2A59D84DC1B6A64E36BC7E9FE29AF6CA3A87AA398F";
const G1_Y_6: &str = "0181E06254E9432CA7C3FD61CEA96175D6D99BB78A1FE848F354D7FB727A50A7F3EFBEF595609BA9B3C03EA3FB3DFC79";
const G1_X_7: &str = "00B9EACABE8A2BD0BDDF7ED1DC14BAEAAAEDC63EF62ECECFACFF0C2A59D84DC1B6A64E36BC7E9FE29AF6CA3A87AA398F";
const G1_Y_7: &str = "002C59E3C2DBCDBE1E77085E9DF7E7C543493E3B76D52B462B9E8A34478EF758231B9E4E9A9F6456D148815C04C20388";
const G1_X_8: &str = "00B9EACABE8A2BD0BDDF7ED1DC14BAEAAAEDC63EF62ECECFACFF0C2A59D84DC1B6A64E36BC7E9FE29AF6CA3A87AA398F";
const G1_Y_8: &str = "002C59E3C2DBCDBE1E77085E9DF7E7C543493E3B76D52B462B9E8A34478EF758231B9E4E9A9F6456D148815C04C20388";
// G2 generator
const G2_X_C0: &str = "0079032CD5D4C0484673FF7F131B151A378E02C6F013AAF4D2A92BB201DAF98AEC2018CFD62F9EB8B7BAF3B049D35728";
const G2_X_C1: &str = "00D48600562B8D313FB188B42B196A49FED48CFFB591CE7134B1F73D9ABEECA147138BA1327C4C6D7FADA61704B95626";
const G2_Y_C0: &str = "00C67FF0068B87A0B9DEED1804C9BEF06814DD79BAD0A05DBA81AEA18CD83E6766EC6EE3D3E20B58044C749FC44366B8";
const G2_Y_C1: &str = "0137475DC960B837006D974B800104C9BA9DD100344BD2A363A89B7C15071F6F63A644A52C9B9BF5AFD0E6C1D7E1FAFF";

lazy_static! {
    pub static ref G2_GENERATOR: G2Projective = g2_generator();
    pub static ref G1_GENERATOR1: G1Projective = {
        let x = Fq::from_repr(read_bigint384_const(G1_X_1));
        let y = Fq::from_repr(read_bigint384_const(G1_Y_1));
        G1Affine::new(x, y, false).into_projective()
    };
    pub static ref G1_GENERATOR2: G1Projective = {
        let x = Fq::from_repr(read_bigint384_const(G1_X_2));
        let y = Fq::from_repr(read_bigint384_const(G1_Y_2));
        G1Affine::new(x, y, false).into_projective()
    };
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CRHWindow;

// TODO: Verify parameters. 133 * 8 = 1064 bits.
// Our fixed-length input is 1064 bits.
impl PedersenWindow for CRHWindow {
    const WINDOW_SIZE: usize = 133;
    const NUM_WINDOWS: usize = 8;
}

fn read_bigint384_const(constant: &str) -> BigInteger384 {
    let mut v = hex::decode(constant).unwrap();
    v.reverse();
    BigInteger384::read(&mut &v[..]).unwrap()
}

pub fn setup_crh<W: PedersenWindow>() -> PedersenParameters<G1Projective> {
    let mut base_generators = vec![];
    let x = Fq::from_repr(read_bigint384_const(G1_X_1));
    let y = Fq::from_repr(read_bigint384_const(G1_Y_1));
    base_generators.push(G1Affine::new(x, y, false));
    let x = Fq::from_repr(read_bigint384_const(G1_X_2));
    let y = Fq::from_repr(read_bigint384_const(G1_Y_2));
    base_generators.push(G1Affine::new(x, y, false));
    let x = Fq::from_repr(read_bigint384_const(G1_X_3));
    let y = Fq::from_repr(read_bigint384_const(G1_Y_3));
    base_generators.push(G1Affine::new(x, y, false));
    let x = Fq::from_repr(read_bigint384_const(G1_X_4));
    let y = Fq::from_repr(read_bigint384_const(G1_Y_4));
    base_generators.push(G1Affine::new(x, y, false));
    let x = Fq::from_repr(read_bigint384_const(G1_X_5));
    let y = Fq::from_repr(read_bigint384_const(G1_Y_5));
    base_generators.push(G1Affine::new(x, y, false));
    let x = Fq::from_repr(read_bigint384_const(G1_X_6));
    let y = Fq::from_repr(read_bigint384_const(G1_Y_6));
    base_generators.push(G1Affine::new(x, y, false));
    let x = Fq::from_repr(read_bigint384_const(G1_X_7));
    let y = Fq::from_repr(read_bigint384_const(G1_Y_7));
    base_generators.push(G1Affine::new(x, y, false));
    let x = Fq::from_repr(read_bigint384_const(G1_X_8));
    let y = Fq::from_repr(read_bigint384_const(G1_Y_8));
    base_generators.push(G1Affine::new(x, y, false));
    assert!(W::NUM_WINDOWS <= base_generators.len());

    let mut generators = Vec::new();
    for i in 0..W::NUM_WINDOWS {
        let mut generators_for_segment = Vec::new();
        let mut base = base_generators[i].into_projective();
        for _ in 0..W::WINDOW_SIZE {
            generators_for_segment.push(base);
            for _ in 0..4 {
                base.double_in_place();
            }
        }
        generators.push(generators_for_segment);
    }

    PedersenParameters { generators }
}

fn g2_generator() -> G2Projective {
    let x_c0 = read_bigint384_const(G2_X_C0);
    let x_c1 = read_bigint384_const(G2_X_C1);
    let y_c0 = read_bigint384_const(G2_Y_C0);
    let y_c1 = read_bigint384_const(G2_Y_C1);

    let x = Fq2::new(Fq::from_repr(x_c0), Fq::from_repr(x_c1));
    let y = Fq2::new(Fq::from_repr(y_c0), Fq::from_repr(y_c1));
    G2Affine::new(x, y, false).into_projective()
}
