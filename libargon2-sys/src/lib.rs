#![allow(dead_code)]

use libc::{c_int, size_t};

extern {
    fn argon2d_hash_raw_flags(t_cost: u32, m_cost: u32,
        parallelism: u32, pwd: *const u8,
        pwdlen: size_t, salt: *const u8,
        saltlen: size_t, hash: *mut u8,
        hashlen: size_t, flags: u32) -> c_int;
}

pub fn argon2d_hash(t_cost: u32, m_cost: u32, parallelism: u32, pwd: &[u8], salt: &[u8], out: &mut [u8], flags: u32) {
    unsafe { argon2d_hash_raw_flags(t_cost, m_cost, parallelism,pwd.as_ptr(), pwd.len(), salt.as_ptr(), salt.len(), out.as_mut_ptr(), out.len(), flags) };
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
        argon2d_hash(1, 512, 1,b"test", b"nimiqrocks!", &mut res, 0);
        assert_eq!(res, [140, 37, 159, 220, 194, 173, 103, 153, 223, 114, 140, 17, 232, 149, 163, 54, 158, 157, 186, 230, 163, 22, 110, 188, 59, 53, 51, 153, 252, 86, 85, 36]);
    }
}
