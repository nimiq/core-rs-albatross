use beserial::ReadBytesExt;
use nimiq_utils::crc::Crc32Computer;
use std::io;

pub struct ReaderComputeCrc32<'a, T: 'a + ReadBytesExt> {
    reader: &'a mut T,
    pub(super) crc32: Crc32Computer,
    pub(super) at_checksum: bool,
    pub(super) length: usize,
}

impl<'a, T: ReadBytesExt> ReaderComputeCrc32<'a, T> {
    pub fn new(reader: &'a mut T) -> ReaderComputeCrc32<T> {
        ReaderComputeCrc32 {
            reader,
            crc32: Crc32Computer::default(),
            at_checksum: false,
            length: 0,
        }
    }
}

impl<'a, T: ReadBytesExt> io::Read for ReaderComputeCrc32<'a, T> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, io::Error> {
        let size = self.reader.read(buf)?;
        if size > 0 {
            if self.at_checksum {
                // We're at the checksum, so'll just ignore it
                let zeros = [0u8; 4];
                self.crc32.update(&zeros);
            } else {
                self.crc32.update(&buf[..size]);
            }
        }
        self.length += size;
        Ok(size)
    }
}
