#[doc(hidden)]
pub extern crate hex;

#[doc(hidden)]
pub extern crate serde;

#[doc(hidden)]
pub extern crate serde_big_array;

#[doc(hidden)]
pub extern crate nimiq_serde;

#[macro_export]
macro_rules! create_typed_array {
    ($name: ident, $t: ty, $len: expr) => {
        #[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Hash)]
        pub struct $name(pub [$t; $len]);

        impl<'a> From<&'a [$t]> for $name {
            fn from(slice: &'a [$t]) -> Self {
                assert_eq!(
                    slice.len(),
                    $len,
                    "Tried to create instance with slice of wrong length"
                );
                let mut a = [0 as $t; $len];
                a.clone_from_slice(&slice[0..$len]);
                $name(a)
            }
        }

        impl Default for $name {
            fn default() -> Self {
                $name([<$t>::default(); $len])
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
macro_rules! add_serialization_fns_typed_arr {
    ($name: ident, $len: expr) => {
        impl ::nimiq_macros::serde::Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: ::nimiq_macros::serde::Serializer,
            {
                if serializer.is_human_readable() {
                    ::nimiq_macros::serde::Serialize::serialize(
                        &::nimiq_macros::hex::encode(&self.0),
                        serializer,
                    )
                } else {
                    ::nimiq_macros::serde_big_array::BigArray::serialize(&self.0, serializer)
                }
            }
        }

        impl<'de> ::nimiq_macros::serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: ::nimiq_macros::serde::de::Deserializer<'de>,
            {
                let data: [u8; $len] = if deserializer.is_human_readable() {
                    let s: Cow<'de, str> =
                        ::nimiq_macros::serde::Deserialize::deserialize(deserializer)?;
                    ::nimiq_macros::hex::decode(&s[..])
                        .map_err(|_| {
                            <D::Error as ::nimiq_macros::serde::de::Error>::custom(
                                "Could not parse hex string",
                            )
                        })?
                        .try_into()
                        .map_err(|_| {
                            <D::Error as ::nimiq_macros::serde::de::Error>::custom(
                                "Could not parse hex string: invalid length",
                            )
                        })?
                } else {
                    ::nimiq_macros::serde_big_array::BigArray::deserialize(deserializer)?
                };
                Ok($name(data))
            }
        }
    };
}

#[macro_export]
macro_rules! add_hex_io_fns_typed_arr {
    ($name: ident, $len: expr) => {
        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                f.write_str(&::nimiq_macros::hex::encode(&self.0))
            }
        }

        impl $name {
            pub fn to_hex(&self) -> String {
                ::nimiq_macros::hex::encode(&self.0)
            }

            pub fn to_short_str(&self) -> String {
                ::nimiq_macros::hex::encode(&self.0[0..5])
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = ::nimiq_macros::hex::FromHexError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                use ::nimiq_macros::hex::FromHex;

                let vec = Vec::from_hex(s)?;
                if vec.len() == $len {
                    Ok($name::from(&vec[..]))
                } else {
                    Err(::nimiq_macros::hex::FromHexError::InvalidStringLength)
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
                f.write_str(&::nimiq_macros::hex::encode(&self.0))
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

#[macro_export]
macro_rules! store_waker {
    ($self: ident, $field: ident, $cx: expr) => {
        match &mut $self.$field {
            Some(waker) if !waker.will_wake($cx.waker()) => *waker = $cx.waker().clone(),
            None => $self.$field = Some($cx.waker().clone()),
            _ => {}
        }
    };
}
