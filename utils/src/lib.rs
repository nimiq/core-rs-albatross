// TODO: Remove when stabilized.
#![feature(div_duration)]

#[cfg(feature = "beserial_derive")]
#[macro_use]
extern crate beserial_derive;
#[cfg(feature = "log")]
#[macro_use]
extern crate log;

#[cfg(feature = "crc")]
pub mod crc;
#[cfg(feature = "key-store")]
pub mod file_store;
#[cfg(feature = "hash-rng")]
pub mod hash_rng;
#[cfg(feature = "iterators")]
pub mod iterators;
#[cfg(feature = "key-rng")]
pub mod key_rng;
// #[cfg(feature = "locking")]
// pub mod locking;
#[cfg(feature = "math")]
pub mod math;
#[cfg(feature = "merkle")]
pub mod merkle;
#[cfg(feature = "mutable-once")]
pub mod mutable_once;
#[cfg(feature = "observer")]
pub mod observer;
#[cfg(feature = "otp")]
pub mod otp;
#[cfg(feature = "rate-limit")]
pub mod rate_limit;
#[cfg(feature = "tagged-signing")]
pub mod tagged_signing;
#[cfg(feature = "throttled-queue")]
pub mod throttled_queue;
#[cfg(feature = "time")]
pub mod time;
// #[cfg(feature = "timers")]
// pub mod timers;
#[cfg(feature = "unique-id")]
pub mod unique_id;
#[cfg(feature = "unique-ptr")]
pub mod unique_ptr;
