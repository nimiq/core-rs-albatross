pub trait Hash<H: Merge> {
    fn hash(&self, prefix: u64) -> H;
}

pub trait Merge {
    fn empty(prefix: u64) -> Self;
    fn merge(&self, other: &Self, prefix: u64) -> Self;
}
