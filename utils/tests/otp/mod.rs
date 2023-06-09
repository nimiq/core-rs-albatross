use nimiq_test_log::test;
use nimiq_utils::otp::*;
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize, Eq, PartialEq, Clone)]
struct DummyU32 {
    value: u32,
    checksum: u32,
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
        Err(e) => panic!("{:?}", e),
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
fn create_locked_unchecked() {
    let secret = 12345u32.to_be_bytes();
    let password = "password";
    let wrong_password = "wrong_password";

    let lock = match OtpLock::locked_with_defaults(secret, password.as_bytes()) {
        Ok(l) => l,
        Err(e) => panic!("{:?}", e),
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
