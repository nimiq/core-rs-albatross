pub trait Hash<H: Merge> {
    fn hash(&self, prefix: usize) -> H;
}

pub trait Merge {
    fn empty(prefix: usize) -> Self;
    fn merge(&self, other: &Self, prefix: usize) -> Self;
}
