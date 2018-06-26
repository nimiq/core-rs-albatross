macro_rules! create_typed_array {
    ($name: ident, $t: ty, $len: expr) => {
        #[repr(C)]
        #[derive(Default,Clone,PartialEq,PartialOrd,Eq,Ord,Debug)]
        pub struct $name ([$t; $len]);

        impl<'a> From<&'a [$t]> for $name {
            fn from(slice: &'a [$t]) -> Self {
                assert_eq!(slice.len(), $len, "Tried to create instance with slice of wrong length");
                let mut a = [0 as $t; $len];
                a.clone_from_slice(&slice[0..$len]);
                return $name(a);
            }
        }

        impl ::beserial::Deserialize for $name {
            fn deserialize<R: ::beserial::ReadBytesExt>(reader: &mut R) -> ::std::io::Result<Self> {
                let mut a = [0 as $t; $len];
                reader.read_exact(&mut a[..])?;
                return Ok($name(a))
            }
        }

        impl ::beserial::Serialize for $name {
            fn serialize<W: ::beserial::WriteBytesExt>(&self, writer: &mut W) -> ::std::io::Result<usize> {
                writer.write(&self.0)?;
                return Ok($len);
            }

            fn serialized_size(&self) -> usize {
                return $len;
            }
        }

        impl From<[$t; $len]> for $name {
            fn from(arr: [$t; $len]) -> Self {
                return $name(arr);
            }
        }

        impl From<$name> for [$t; $len] {
            fn from(i: $name) -> [$t; $len] {
                return i.0;
            }
        }

        impl $name {
            pub const SIZE: usize = $len;
            pub fn len() -> usize { $len }
        }
    };
}

macro_rules! add_hex_io_fns_typed_arr {
    ($name: ident, $len: expr) => {
        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                return f.write_str(&::hex::encode(&self.0));
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = ::hex::FromHexError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let vec = Vec::from_hex(s)?;
                if vec.len() == $len {
                    return Ok($name::from(&vec[..]));
                } else {
                    return Err(::hex::FromHexError::InvalidStringLength);
                }
            }
        }

        impl From<&'static str> for $name {
            fn from(s: &'static str) -> Self {
                return s.parse().unwrap();
            }
        }
    };
}

macro_rules! add_hash_trait_typed_arr {
    ($name: ident) => {
        impl<H> Hash<H> for $name where H: Hasher {
            fn hash(&self, state: &mut H) {
                state.write(&self.0);
            }
        }
    };
}

macro_rules! add_hash_trait_arr {
    ($t: ty) => {
        impl<H> Hash<H> for $t where H: Hasher {
            fn hash(&self, state: &mut H) {
                state.write(&self[..]);
            }
        }
    };
}

macro_rules! implement_simple_add_sum_traits {
    ($name: ident, $identity: expr) => {
        impl<'a, 'b> Add<&'b $name> for &'a $name {
            type Output = $name;
            fn add(self, other: &'b $name) -> $name {
                $name(self.0 + other.0)
            }
        }
        impl<'b> Add<&'b $name> for $name {
            type Output = $name;
            fn add(self, rhs: &'b $name) -> $name {
                &self + rhs
            }
        }

        impl<'a> Add<$name> for &'a $name {
            type Output = $name;
            fn add(self, rhs: $name) -> $name {
                self + &rhs
            }
        }

        impl Add<$name> for $name {
            type Output = $name;
            fn add(self, rhs: $name) -> $name {
                &self + &rhs
            }
        }

        impl<T> Sum<T> for $name
            where
                T: Borrow<$name>
        {
            fn sum<I>(iter: I) -> Self
                where
                    I: Iterator<Item = T>
            {
                $name(iter.fold($identity, |acc, item| acc + item.borrow().0))
            }
        }
    }
}
