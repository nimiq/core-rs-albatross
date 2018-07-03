use beserial::{Deserialize, Serialize};
use consensus::base::primitive::hash::Argon2dHash;

#[derive(Default, Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Debug, Serialize, Deserialize)]
pub struct TargetCompact(u32);

create_typed_array!(Target, u8, 32);

impl From<TargetCompact> for u32 {
    fn from(t: TargetCompact) -> Self { t.0 }
}

impl From<u32> for TargetCompact {
    fn from(u: u32) -> Self { TargetCompact(u) }
}

impl From<TargetCompact> for Target {
    fn from(t: TargetCompact) -> Self {
        let mut val = [0u8; 32];
        let shift_bytes: usize = ((t.0 >> 24) - 3) as usize;
        let value = t.0 & 0xffffff;
        val[32 - shift_bytes - 1] = (value & 0xff) as u8;
        val[32 - shift_bytes - 2] = ((value >> 8) & 0xff) as u8;
        val[32 - shift_bytes - 3] = ((value >> 16) & 0xff) as u8;
        return Target(val);
    }
}

impl From<Target> for TargetCompact {
    fn from(target: Target) -> Self {
        let mut first_byte = 0;
        for i in 0..target.0.len() {
            if target.0[i] > 0 {
                first_byte = i;
                break;
            }
        }
        if target.0[first_byte] >= 0x80 {
            first_byte -= 1;
        }
        let shift_bytes = 32 - first_byte;

        return TargetCompact(((shift_bytes as u32) << 24) + ((target.0[first_byte] as u32) << 16) + ((target.0[first_byte + 1] as u32) << 8) + target.0[first_byte + 2] as u32);
    }
}

impl Target {
    pub fn is_reached_by(&self, hash: &Argon2dHash) -> bool {
        let reached = Target::from(hash.as_bytes());
        return &reached < self;
    }
}
