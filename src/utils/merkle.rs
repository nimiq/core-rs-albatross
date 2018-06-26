use consensus::base::primitive::hash::Hash;
use consensus::base::primitive::hash::Hasher;

pub fn compute_root<D: Hasher, T: Hash<D>>(values: &Vec<T>) -> D::Output where D::Output: Hash<D> {
    return compute_root_from_slice(values.as_slice());
}

pub fn compute_root_from_slice<D: Hasher, T: Hash<D>>(values: &[T]) -> D::Output where D::Output: Hash<D> {
    let mut hasher = D::default();
    match values.len() {
        0 => {
            hasher.write(&[0u8]);
        },
        1 => {
            values[0].hash(&mut hasher);
        },
        len => {
            let mid = (len + 1) / 2; // Equivalent to round(len / 2.0)
            let left_hash = compute_root_from_slice(&values[..mid]);
            let right_hash = compute_root_from_slice(&values[mid..]);
            left_hash.hash(&mut hasher);
            right_hash.hash(&mut hasher);
        },
    };
    return hasher.finish();
}
