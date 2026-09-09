use alloc::vec::Vec;
use bytemuck::{Pod, Zeroable};

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(C, align(16))]
struct Chunk([u8; 16]);

pub struct Aligned {
    chunks: Vec<Chunk>,
    len: usize,
}

impl Aligned {
    #[must_use]
    pub fn zeroed(len: usize) -> Self {
        Self {
            chunks: alloc::vec![Chunk([0; 16]); len.div_ceil(16)],
            len,
        }
    }
    #[must_use]
    pub fn as_slice(&mut self) -> &[u8] {
        let len = self.len;
        bytemuck::cast_slice::<Chunk, u8>(&mut self.chunks)
            .get(..len)
            .unwrap_or(&[])
    }
    #[must_use]
    pub fn as_mut_slice(&mut self) -> &[u8] {
        let len = self.len;
        bytemuck::cast_slice_mut::<Chunk, u8>(&mut self.chunks)
            .get_mut(..len)
            .unwrap_or(&mut [])
    }
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}
