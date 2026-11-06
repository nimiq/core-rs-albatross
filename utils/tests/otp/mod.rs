use nimiq_serde::{Deserialize as _, Serialize as _};
use nimiq_test_log::test;
use nimiq_utils::otp::*;
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize, Eq, PartialEq, Clone)]
struct DummyU32 {
    // Fixed width: as varints, a wrong password verifies in 1 of 258111 runs.
    #[serde(with = "nimiq_serde::fixint::be")]
    value: u32,
    #[serde(with = "nimiq_serde::fixint::be")]
    checksum: u32,
}

impl Zeroize for DummyU32 {
    fn zeroize(&mut self) {
        self.value.zeroize();
        self.checksum.zeroize();
    }
}

impl DummyU32 {
    fn new(value: u32) -> Self {
        DummyU32 {
            checksum: value,
            value,
        }
    }
}

impl Verify for DummyU32 {
    fn verify(&self) -> bool {
        self.value == self.checksum
    }
}

#[test]
fn create_unlocked_checked() {
    let secret = DummyU32::new(12345);
    let password = "password";
    let wrong_password = "wrong_password";

    let lock = match OtpLock::unlocked_with_defaults(secret.clone(), password.as_bytes()) {
        Ok(l) => l,
        Err(e) => panic!("{e:?}"),
    };

    assert!(lock.is_unlocked());

    if let OtpLock::Unlocked(unlocked) = lock {
        assert!(unlocked.eq(&secret));

        // Try locking.
        let locked = Unlocked::lock(unlocked);

        // Try unlocking.
        let mut unlocked = locked.unlock(wrong_password.as_bytes());
        assert!(unlocked.is_err());

        let locked = unlocked.err().unwrap();
        unlocked = locked.unlock(password.as_bytes());

        assert!(unlocked.is_ok());
        let unlocked = unlocked.ok().unwrap();
        assert!(unlocked.eq(&secret));
    }
}

#[test]
fn unlock_rejects_trailing_bytes() {
    let password = "password";
    let bytes = Locked::with_defaults([7u8; 8], password.as_bytes())
        .unwrap()
        .serialize_to_vec();

    // The serialized lock does not depend on its type parameter.
    let locked = Locked::<[u8; 8]>::deserialize_all(&bytes).unwrap();
    assert!(locked.unlock_unchecked(password.as_bytes()).is_ok());

    // Read as a shorter type, the correct password leaves bytes unread.
    let locked = Locked::<[u8; 4]>::deserialize_all(&bytes).unwrap();
    assert!(locked.unlock_unchecked(password.as_bytes()).is_err());
}

#[test]
fn create_locked_unchecked() {
    let secret = 12345u32.to_be_bytes();
    let password = "password";
    let wrong_password = "wrong_password";

    let lock = match OtpLock::locked_with_defaults(secret, password.as_bytes()) {
        Ok(l) => l,
        Err(e) => panic!("{e:?}"),
    };

    assert!(lock.is_locked());

    let locked = lock.locked();

    // Unlock with wrong password.
    let unlocked = locked.unlock_unchecked(wrong_password.as_bytes());
    assert!(unlocked.is_ok());
    let unlocked = unlocked.ok().unwrap();
    assert!(!unlocked.eq(&secret));

    let unlocked = Unlocked::into_otp_lock(unlocked);
    assert!(unlocked.unlocked_ref().is_some());

    let locked = unlocked.lock();
    assert!(locked.unlocked_ref().is_none());

    // Unlock with correct password.
    let unlocked = locked.locked().unlock_unchecked(password.as_bytes());
    assert!(unlocked.is_ok());
    let unlocked = unlocked.ok().unwrap();
    assert!(unlocked.eq(&secret));
}
