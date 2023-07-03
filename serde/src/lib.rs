use std::error::Error;
use std::fmt;
use std::io;
use std::io::Write;
use std::ops;

use serde::{
    de::{Deserializer, Error as _},
    ser::Serializer,
};
use serde_big_array::BigArray;

pub use postcard::fixint;
pub use serde_derive::Deserialize;
pub use serde_derive::Serialize;

#[derive(Eq, PartialEq)]
pub struct DeserializeError(postcard::Error);

impl DeserializeError {
    pub fn bad_enum() -> DeserializeError {
        DeserializeError(postcard::Error::DeserializeBadEnum)
    }
}

impl fmt::Debug for DeserializeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for DeserializeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Error for DeserializeError {}

/// The Nimiq human readable array serialization helper trait
///
/// ```
/// # use serde::{Serialize, Deserialize};
/// # use nimiq_serde::HexArray;
/// #[derive(Serialize, Deserialize)]
/// struct S {
///     #[serde(with = "HexArray")]
///     arr: [u8; 64],
/// }
/// ```
pub trait HexArray<'de>: Sized {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer;
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>;
}

impl<'de, const N: usize> HexArray<'de> for [u8; N] {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            serializer.serialize_str(&hex::encode(self))
        } else {
            BigArray::serialize(self, serializer)
        }
    }

    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let s: String = serde::Deserialize::deserialize(deserializer)?;
            let mut out = [0u8; N];
            hex::decode_to_slice(s, &mut out)
                .map_err(|_| D::Error::custom("Couldn't decode hex string"))?;
            Ok(out)
        } else {
            BigArray::deserialize(deserializer)
        }
    }
}

/// The Nimiq wrapper for serializing std::ops::RangeFrom
#[derive(Clone, Debug)]
pub struct SerRangeFrom<T>(pub ops::RangeFrom<T>);

impl<T: Serialize> serde::Serialize for SerRangeFrom<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serde::Serialize::serialize(&self.0.start, serializer)
    }
}

impl<'de, T: Deserialize> serde::Deserialize<'de> for SerRangeFrom<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let start: T = serde::Deserialize::deserialize(deserializer)?;
        Ok(Self(std::ops::RangeFrom { start }))
    }
}

pub trait Serialize: serde::Serialize {
    fn serialize_to_writer<W: Write>(&self, writer: &mut W) -> io::Result<usize> {
        struct Wrapper<'a, 'b, W: Write> {
            inner: &'a mut W,
            written: &'b mut usize,
            error: &'b mut Option<io::Error>,
        }
        impl<'a, 'b, W: Write> postcard::ser_flavors::Flavor for Wrapper<'a, 'b, W> {
            type Output = ();
            fn try_push(&mut self, data: u8) -> postcard::Result<()> {
                *self.written += 1;
                self.try_extend(&[data])
            }
            fn try_extend(&mut self, data: &[u8]) -> postcard::Result<()> {
                assert!(self.error.is_none());
                *self.written += data.len();
                match self.inner.write_all(data) {
                    Ok(()) => Ok(()),
                    Err(e) => {
                        *self.error = Some(e);
                        Err(postcard::Error::SerializeBufferFull)
                    },
                }
            }
            fn finalize(self) -> postcard::Result<()> {
                Ok(())
            }
        }
        let mut written = 0;
        let mut error = None;
        let wrapper = Wrapper {
            inner: writer,
            written: &mut written,
            error: &mut error,
        };
        match postcard::serialize_with_flavor(self, wrapper) {
            Ok(()) => Ok(written),
            Err(postcard::Error::SerializeBufferFull) => Err(error.unwrap()),
            Err(e) => panic!("unexpected error {}", e),
        }
    }
    fn serialize<W: Write>(&self, writer: &mut W) -> io::Result<usize> {
        self.serialize_to_writer(writer)
    }
    fn serialize_to_vec(&self) -> Vec<u8> {
        postcard::to_allocvec(self).unwrap()
    }
    fn serialized_size(&self) -> usize {
        let size = postcard::ser_flavors::Size::default();
        postcard::serialize_with_flavor(self, size).unwrap()
    }
}

pub trait Deserialize: serde::de::DeserializeOwned {
    fn deserialize_from_vec(bytes: &[u8]) -> Result<Self, DeserializeError> {
        postcard::from_bytes(bytes).map_err(DeserializeError)
    }
    fn deserialize_take(bytes: &[u8]) -> Result<(Self, &[u8]), DeserializeError> {
        postcard::take_from_bytes(bytes).map_err(DeserializeError)
    }
}

impl<T: serde::Serialize> Serialize for T {}

impl<T: serde::de::DeserializeOwned> Deserialize for T {}
