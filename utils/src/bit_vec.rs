use std::cmp::min;
use bit_vec::{BitVec, BitBlock};

pub trait IntoChunkedBitVecIterator<'a, B: 'a = u32> {
    fn chunks(&'a self, chunk_size: usize) -> ChunkedBitVecIterator<'a, B>;
}

#[derive(Clone)]
pub struct ChunkedBitVecIterator<'a, B: 'a = u32> {
    bit_vec: &'a BitVec<B>,
    pos: usize,
    chunk_size: usize,
}

impl<'a, B: BitBlock> Iterator for ChunkedBitVecIterator<'a, B> {
    type Item = usize;

    #[inline]
    fn next(&mut self) -> Option<usize> {
        if self.pos >= self.bit_vec.len() {
            return None
        }
        let end_pos = min(self.pos + self.chunk_size, self.bit_vec.len());
        let ret = (self.pos..end_pos).fold(0, |acc, pos| (acc << 1) | (self.bit_vec.get(pos).unwrap() as usize) as usize);
        self.pos += self.chunk_size;
        Some(ret)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let exact_fit = (self.bit_vec.len() % self.chunk_size) == 0;
        let size = (self.bit_vec.len() / self.chunk_size) + (if exact_fit { 0 } else { 1 });
        (size, Some(size))
    }
}

impl<'a, B: BitBlock> ExactSizeIterator for ChunkedBitVecIterator<'a, B> {}

impl<'a, B: 'a> IntoChunkedBitVecIterator<'a, B> for BitVec<B> {
    fn chunks(&'a self, chunk_size: usize) -> ChunkedBitVecIterator<'a, B> {
        ChunkedBitVecIterator {
            bit_vec: &self,
            pos: 0,
            chunk_size,
        }
    }
}
