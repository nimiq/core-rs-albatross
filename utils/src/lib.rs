// TODO: Remove when stabilized.
#![feature(div_duration)]

#[cfg(feature = "log")]
#[macro_use]
extern crate log;

#[cfg(feature = "beserial_derive")]
#[macro_use]
extern crate beserial_derive;

#[cfg(feature = "crc")]
pub mod crc;
#[cfg(feature = "key-store")]
pub mod key_store;
#[cfg(feature = "merkle")]
pub mod merkle;
#[cfg(feature = "locking")]
pub mod locking;
#[cfg(feature = "bit-vec")]
pub mod bit_vec;
#[cfg(feature = "observer")]
pub mod observer;
#[cfg(feature = "timers")]
pub mod timers;
#[cfg(feature = "unique-ptr")]
pub mod unique_ptr;
#[cfg(feature = "iterators")]
pub mod iterators;
#[cfg(feature = "mutable-once")]
pub mod mutable_once;
#[cfg(feature = "time")]
pub mod time;
#[cfg(feature = "throttled-queue")]
pub mod throttled_queue;
#[cfg(feature = "rate-limit")]
pub mod rate_limit;
#[cfg(feature = "sleep-on-error")]
pub mod sleep_on_error;
#[cfg(feature = "unique-id")]
pub mod unique_id;
#[cfg(feature = "otp")]
pub mod otp;
#[cfg(feature = "math")]
pub mod math;
#[cfg(feature = "key-rng")]
pub mod key_rng;
#[cfg(feature = "hash-rng")]
pub mod hash_rng;
