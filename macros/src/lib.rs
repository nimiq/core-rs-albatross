#[macro_export]
macro_rules! create_typed_array {
    ($name: ident, $t: ty, $len: expr) => {
        #[repr(C)]
        #[derive(Default, Clone, PartialEq, PartialOrd, Eq, Ord, Hash)]
        pub struct $name([$t; $len]);

        impl<'a> From<&'a [$t]> for $name {
            fn from(slice: &'a [$t]) -> Self {
                assert_eq!(slice.len(), $len, "Tried to create instance with slice of wrong length");
                let mut a = [0 as $t; $len];
                a.clone_from_slice(&slice[0..$len]);
                $name(a)
            }
        }

        impl ::beserial::Deserialize for $name {
            fn deserialize<R: ::beserial::ReadBytesExt>(reader: &mut R) -> Result<Self, ::beserial::SerializingError> {
                let mut a = [0 as $t; $len];
                reader.read_exact(&mut a[..])?;
                Ok($name(a))
            }
        }

        impl ::beserial::Serialize for $name {
            fn serialize<W: ::beserial::WriteBytesExt>(&self, writer: &mut W) -> Result<usize, ::beserial::SerializingError> {
                writer.write_all(&self.0)?;
                Ok($len)
            }

            fn serialized_size(&self) -> usize {
                $len
            }
        }

        impl From<[$t; $len]> for $name {
            fn from(arr: [$t; $len]) -> Self {
                $name(arr)
            }
        }

        impl From<$name> for [$t; $len] {
            fn from(i: $name) -> [$t; $len] {
                i.0
            }
        }

        impl AsRef<[$t]> for $name {
            fn as_ref(&self) -> &[$t] {
                &self.0
            }
        }

        impl AsMut<[$t]> for $name {
            fn as_mut(&mut self) -> &mut [$t] {
                &mut self.0
            }
        }

        impl $name {
            pub const SIZE: usize = $len;
            #[inline]
            pub fn len() -> usize {
                $len
            }

            pub fn as_slice(&self) -> &[$t] {
                &self.0
            }
        }
    };
}

#[macro_export]
macro_rules! add_hex_io_fns_typed_arr {
    ($name: ident, $len: expr) => {
        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                f.write_str(&::hex::encode(&self.0))
            }
        }

        impl $name {
            pub fn to_hex(&self) -> String {
                ::hex::encode(&self.0)
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = ::hex::FromHexError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                use ::hex::FromHex;

                let vec = Vec::from_hex(s)?;
                if vec.len() == $len {
                    Ok($name::from(&vec[..]))
                } else {
                    Err(::hex::FromHexError::InvalidStringLength)
                }
            }
        }

        impl From<&'static str> for $name {
            fn from(s: &'static str) -> Self {
                s.parse().unwrap()
            }
        }

        impl ::std::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                f.write_str(&::hex::encode(&self.0))
            }
        }
    };
}

#[macro_export]
macro_rules! upgrade_weak {
    ($weak_ref: expr) => {
        if let Some(arc) = $weak_ref.upgrade() {
            arc
        } else {
            return;
        }
    };
}
