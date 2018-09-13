macro_rules! add_hash_trait_arr {
    ($t: ty) => {
        impl SerializeContent for $t {
            fn serialize_content<W: io::Write>(&self, state: &mut W) -> io::Result<usize> {
                state.write(&self[..])?;
                return Ok(self.len());
            }
        }

        impl Hash for $t {}
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
