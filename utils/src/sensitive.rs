use std::{
    fmt,
    ops::{Deref, DerefMut},
};

use serde::{
    de::{Deserialize, Deserializer},
    ser::{Serialize, Serializer},
};

#[derive(Clone, Copy, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Sensitive<T>(pub T);

impl<T> fmt::Debug for Sensitive<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.pad("***")
    }
}

impl<T, U: ?Sized> AsRef<U> for Sensitive<T>
where
    T: AsRef<U>,
{
    fn as_ref(&self) -> &U {
        self.0.as_ref()
    }
}

impl<T> Deref for Sensitive<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Sensitive<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Sensitive<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Sensitive<T>, D::Error> {
        T::deserialize(deserializer).map(Sensitive)
    }
}

impl<T: Serialize> Serialize for Sensitive<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}
