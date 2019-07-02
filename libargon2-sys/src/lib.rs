#![allow(dead_code)]

use libc::{c_int, size_t};

extern {
    fn argon2d_hash_raw_flags(t_cost: u32, m_cost: u32,
        parallelism: u32, pwd: *const u8,
        pwdlen: size_t, salt: *const u8,
        saltlen: size_t, hash: *mut u8,
        hashlen: size_t, flags: u32) -> c_int;
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum Argon2Error {
    InvalidReturnValue = 0,

    Argon2OutputPtrNull = -1,

    Argon2OutputTooShort = -2,
    Argon2OutputTooLong = -3,

    Argon2PwdTooShort = -4,
    Argon2PwdTooLong = -5,

    Argon2SaltTooShort = -6,
    Argon2SaltTooLong = -7,

    Argon2AdTooShort = -8,
    Argon2AdTooLong = -9,

    Argon2SecretTooShort = -10,
    Argon2SecretTooLong = -11,

    Argon2TimeTooSmall = -12,
    Argon2TimeTooLarge = -13,

    Argon2MemoryTooLittle = -14,
    Argon2MemoryTooMuch = -15,

    Argon2LanesTooFew = -16,
    Argon2LanesTooMany = -17,

    Argon2PwdPtrMismatch = -18,    /* NULL ptr with non-zero length */
    Argon2SaltPtrMismatch = -19,   /* NULL ptr with non-zero length */
    Argon2SecretPtrMismatch = -20, /* NULL ptr with non-zero length */
    Argon2AdPtrMismatch = -21,     /* NULL ptr with non-zero length */

    Argon2MemoryAllocationError = -22,

    Argon2FreeMemoryCbkNull = -23,
    Argon2AllocateMemoryCbkNull = -24,

    Argon2IncorrectParameter = -25,
    Argon2IncorrectType = -26,

    Argon2OutPtrMismatch = -27,

    Argon2ThreadsTooFew = -28,
    Argon2ThreadsTooMany = -29,

    Argon2MissingArgs = -30,

    Argon2ThreadFail = -33,

    Argon2VerifyMismatch = -35,
}

impl From<c_int> for Argon2Error {
    fn from(value: c_int) -> Self {
        match value {
            -1 => Argon2Error::Argon2OutputPtrNull,

            -2 => Argon2Error::Argon2OutputTooShort,
            -3 => Argon2Error::Argon2OutputTooLong,

            -4 => Argon2Error::Argon2PwdTooShort,
            -5 => Argon2Error::Argon2PwdTooLong,

            -6 => Argon2Error::Argon2SaltTooShort,
            -7 => Argon2Error::Argon2SaltTooLong,

            -8 => Argon2Error::Argon2AdTooShort,
            -9 => Argon2Error::Argon2AdTooLong,

            -10 => Argon2Error::Argon2SecretTooShort,
            -11 => Argon2Error::Argon2SecretTooLong,

            -12 => Argon2Error::Argon2TimeTooSmall,
            -13 => Argon2Error::Argon2TimeTooLarge,

            -14 => Argon2Error::Argon2MemoryTooLittle,
            -15 => Argon2Error::Argon2MemoryTooMuch,

            -16 => Argon2Error::Argon2LanesTooFew,
            -17 => Argon2Error::Argon2LanesTooMany,

            -18 => Argon2Error::Argon2PwdPtrMismatch,    /* NULL ptr with non-zero length */
            -19 => Argon2Error::Argon2SaltPtrMismatch,   /* NULL ptr with non-zero length */
            -20 => Argon2Error::Argon2SecretPtrMismatch, /* NULL ptr with non-zero length */
            -21 => Argon2Error::Argon2AdPtrMismatch,     /* NULL ptr with non-zero length */

            -22 => Argon2Error::Argon2MemoryAllocationError,

            -23 => Argon2Error::Argon2FreeMemoryCbkNull,
            -24 => Argon2Error::Argon2AllocateMemoryCbkNull,

            -25 => Argon2Error::Argon2IncorrectParameter,
            -26 => Argon2Error::Argon2IncorrectType,

            -27 => Argon2Error::Argon2OutPtrMismatch,

            -28 => Argon2Error::Argon2ThreadsTooFew,
            -29 => Argon2Error::Argon2ThreadsTooMany,

            -30 => Argon2Error::Argon2MissingArgs,

            -33 => Argon2Error::Argon2ThreadFail,

            -35 => Argon2Error::Argon2VerifyMismatch,

            _ => Argon2Error::InvalidReturnValue,
        }
    }
}

pub fn argon2d_hash(t_cost: u32, m_cost: u32, parallelism: u32, pwd: &[u8], salt: &[u8], out: &mut [u8], flags: u32) -> Result<(), Argon2Error> {
    let return_value = unsafe { argon2d_hash_raw_flags(t_cost, m_cost, parallelism,pwd.as_ptr(), pwd.len(), salt.as_ptr(), salt.len(), out.as_mut_ptr(), out.len(), flags) };
    if return_value == 0 {
        Ok(())
    } else {
        Err(Argon2Error::from(return_value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_call() {
        let mut res = [0u8; 32];
        unsafe { argon2d_hash_raw_flags(1, 512, 1, "test".as_ptr(), 4, "nimiqrocks!".as_ptr(), 11, res.as_mut_ptr(), 32, 0) };
        assert_eq!(res, [140, 37, 159, 220, 194, 173, 103, 153, 223, 114, 140, 17, 232, 149, 163, 54, 158, 157, 186, 230, 163, 22, 110, 188, 59, 53, 51, 153, 252, 86, 85, 36]);
    }

    #[test]
    fn api_call() {
        let mut res = [0u8; 32];
        assert!(argon2d_hash(1, 512, 1,b"test", b"nimiqrocks!", &mut res, 0).is_ok());
        assert_eq!(res, [140, 37, 159, 220, 194, 173, 103, 153, 223, 114, 140, 17, 232, 149, 163, 54, 158, 157, 186, 230, 163, 22, 110, 188, 59, 53, 51, 153, 252, 86, 85, 36]);
    }
}
