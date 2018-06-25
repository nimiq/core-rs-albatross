macro_rules! create_typed_array {
    ($name: ident, $t: ty, $len: expr) => {
        #[repr(C)]
		#[derive(Default,Clone,PartialEq,PartialOrd,Eq,Ord,Debug)]
        pub struct $name ([$t; $len]);

        impl<'a> From<&'a [$t]> for $name {
            fn from(slice: &'a [$t]) -> Self {
                assert!(slice.len() == $len, "Tried to create instance with slice of wrong length");
                let mut a = [0 as $t; $len];
                a.clone_from_slice(&slice[0..$len]);
                return $name(a);
            }
        }

        impl fmt::Display for $name {
			fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
				return f.write_str(&hex::encode(&self.0));
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
            pub fn len() -> usize { $len }
        }

        impl str::FromStr for $name {
			type Err = FromHexError;

			fn from_str(s: &str) -> Result<Self, Self::Err> {
				let vec = Vec::from_hex(s)?;
				if vec.len() == $len {
                    return Ok($name::from(&vec[..]));
				} else {
				    return Err(FromHexError::InvalidStringLength);
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
