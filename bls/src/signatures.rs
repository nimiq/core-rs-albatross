use super::*;

#[derive(Clone, Copy)]
pub struct Signature {
    pub(crate) signature: G1Projective,
}

impl Signature {
    // Map hash to point in G1Projective
    // pub(crate) fn hash_to_g1(h: SigHash) -> G1Projective {
    //     G1Projective::random(&mut ChaChaRng::from_seed(h.into()))
    // }
    pub(crate) fn hash_to_g1(h: SigHash) -> G1Projective {
        loop {
            let x_coordinate: Fq = rand::random();
            let y_coordinate: bool = rand::random();
            let point = G1Affine::get_point_from_x(x_coordinate, y_coordinate);
            if point.is_some() {
                let point = G1Affine::from(point.unwrap());
                let g1 = point.scale_by_cofactor().into_affine();
                return g1.into_projective();
            }
        }
    }
}

impl Eq for Signature {}

impl PartialEq for Signature {
    fn eq(&self, other: &Self) -> bool {
        self.signature.eq(&other.signature)
    }
}
