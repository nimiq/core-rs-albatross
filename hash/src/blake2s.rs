use blake2_rfc::blake2s::Blake2s;

#[derive(Clone)]
pub struct Blake2sWithParameterBlock {
    pub output_size: u8,
    pub key_size: u8,
    pub fan_out: u8,
    pub depth: u8,
    pub leaf_length: u32,
    pub node_offset: u32,
    pub xof_output_size: u16,
    pub node_depth: u8,
    pub inner_length: u8,
    pub salt: [u8; 8],
    pub personalization: [u8; 8],
}

impl Default for Blake2sWithParameterBlock {
    fn default() -> Self {
        Self::new()
    }
}

impl Blake2sWithParameterBlock {
    pub fn new() -> Self {
        Self {
            output_size: 32,
            key_size: 0,
            fan_out: 1,
            depth: 1,
            leaf_length: 0,
            node_offset: 0,
            xof_output_size: 0,
            node_depth: 0,
            inner_length: 0,
            salt: [0; 8],
            personalization: [0; 8],
        }
    }

    pub fn new_blake2x(i: usize, xof_output_size: u16) -> Self {
        Self {
            output_size: 32,
            key_size: 0,
            fan_out: 0,
            depth: 0,
            leaf_length: 32,
            node_offset: i as u32,
            xof_output_size,
            node_depth: 0,
            inner_length: 32,
            salt: [0; 8],
            personalization: [0; 8],
        }
    }

    pub fn parameters(&self) -> [u32; 8] {
        let mut parameters = [0; 8];
        parameters[0] =
            u32::from_le_bytes([self.output_size, self.key_size, self.fan_out, self.depth]);
        parameters[1] = self.leaf_length;
        parameters[2] = self.node_offset;
        parameters[3] = u32::from_le_bytes([
            self.xof_output_size as u8,
            (self.xof_output_size >> 8) as u8,
            self.node_depth,
            self.inner_length,
        ]);

        let salt_bytes_1 = <[u8; 4]>::try_from(&self.salt[0..4]).unwrap();
        let salt_bytes_2 = <[u8; 4]>::try_from(&self.salt[4..8]).unwrap();
        let personalization_bytes_1 = <[u8; 4]>::try_from(&self.personalization[0..4]).unwrap();
        let personalization_bytes_2 = <[u8; 4]>::try_from(&self.personalization[4..8]).unwrap();

        parameters[4] = u32::from_le_bytes(salt_bytes_1);
        parameters[5] = u32::from_le_bytes(salt_bytes_2);
        parameters[6] = u32::from_le_bytes(personalization_bytes_1);
        parameters[7] = u32::from_le_bytes(personalization_bytes_2);

        parameters
    }

    pub fn evaluate(&self, input: &[u8]) -> Vec<u8> {
        let mut b = Blake2s::with_parameter_block(&self.parameters());
        b.update(input);
        b.finalize().as_bytes().into()
    }
}
