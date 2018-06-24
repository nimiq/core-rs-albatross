macro_rules! create_typed_array {
    ($name: ident, $t: ty, $len: expr) => {
        #[derive(PartialEq,Eq)]
        pub struct $name ([$t; $len]);

        impl<'a> From<&'a [$t]> for $name {
            fn from(slice: &'a [$t]) -> Self {
                assert!(slice.len() == $len, "Tried to create instance with slice of wrong length");
                let mut a = [0 as $t; $len];
                a.clone_from_slice(&slice[0..$len]);
                return $name(a);
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
    };
}
