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
            #[inline]
            pub fn len() -> usize { $len }
            pub fn as_bytes(&self) -> &[$t] { &self.0 }
        }
    };
}

macro_rules! hash_typed_array {
    ($name: ident) => {
        impl SerializeContent for $name {
            fn serialize_content<W: io::Write>(&self, state: &mut W) -> io::Result<usize> {
                state.write(&self.0[..])?;
                return Ok(Self::SIZE);
            }
        }

        impl Hash for $name {}
    };
}
