use ark_ec::CurveGroup;
use ark_mnt4_753::{G1Projective as MNT4G1Projective, G2Projective as MNT4G2Projective};
use ark_mnt6_753::{G1Projective as MNT6G1Projective, G2Projective as MNT6G2Projective};
use ark_serialize::CanonicalSerialize;

/// Serializes a G1 point in the MNT4-753 curve.
pub fn serialize_g1_mnt4(point: &MNT4G1Projective) -> [u8; 95] {
    let mut buffer = [0u8; 95];
    CanonicalSerialize::serialize_compressed(&point.into_affine(), &mut buffer[..]).unwrap();
    buffer
}

/// Serializes a G2 point in the MNT4-753 curve.
pub fn serialize_g2_mnt4(point: &MNT4G2Projective) -> [u8; 190] {
    let mut buffer = [0u8; 190];
    CanonicalSerialize::serialize_compressed(&point.into_affine(), &mut buffer[..]).unwrap();
    buffer
}

/// Serializes a G1 point in the MNT6-753 curve.
pub fn serialize_g1_mnt6(point: &MNT6G1Projective) -> [u8; 95] {
    let mut buffer = [0u8; 95];
    CanonicalSerialize::serialize_compressed(&point.into_affine(), &mut buffer[..]).unwrap();
    buffer
}

/// Serializes a G2 point in the MNT6-753 curve.
pub fn serialize_g2_mnt6(point: &MNT6G2Projective) -> [u8; 285] {
    let mut buffer = [0u8; 285];
    CanonicalSerialize::serialize_compressed(&point.into_affine(), &mut buffer[..]).unwrap();
    buffer
}
