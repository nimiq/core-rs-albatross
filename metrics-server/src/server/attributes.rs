use std::fmt;
use std::fmt::Display;
use std::ops::Add;
use std::sync::Arc;

pub trait Attributes: Display {
    #[inline]
    fn is_empty(&self) -> bool;
}

#[derive(Debug)]
pub struct CombinedAttributes<'a> {
    pub vec_attributes: VecAttributes,
    pub cached_attributes: &'a CachedAttributes,
}

impl<'a> CombinedAttributes<'a> {
    #[inline]
    pub fn with_attributes(vec_attributes: VecAttributes, cached_attributes: &'a CachedAttributes) -> Self {
        CombinedAttributes {
            vec_attributes,
            cached_attributes,
        }
    }
}

impl<'a> Attributes for CombinedAttributes<'a> {
    #[inline]
    fn is_empty(&self) -> bool {
        self.vec_attributes.is_empty() && self.cached_attributes.is_empty()
    }
}

impl<'a> Display for CombinedAttributes<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.write_str(&self.vec_attributes.to_string())?;
        if !self.vec_attributes.is_empty() && !self.cached_attributes.is_empty() {
            f.write_str(",")?;
        }
        f.write_str(&self.cached_attributes.to_string())
    }
}

impl<'a> From<&'a CachedAttributes> for CombinedAttributes<'a> {
    fn from(attr: &'a CachedAttributes) -> Self {
        CombinedAttributes {
            vec_attributes: VecAttributes::new(),
            cached_attributes: attr,
        }
    }
}

#[derive(Debug)]
pub struct VecAttributes {
    attributes: Vec<(String, String)>,
}

impl VecAttributes {
    #[inline]
    pub fn new() -> Self {
        VecAttributes {
            attributes: Vec::new(),
        }
    }

    #[inline]
    pub fn with_attributes(attributes: Vec<(String, String)>) -> Self {
        VecAttributes {
            attributes,
        }
    }

    #[inline]
    pub fn add<K: ToString, V: ToString>(&mut self, key: K, value: V) {
        self.attributes.push((key.to_string(), value.to_string()));
    }

    fn build_str(&self) -> String {
        self.attributes
            .iter()
            .map(|(k, v)| format!("{}=\"{}\"", k, v))
            .collect::<Vec<String>>().join(",")
    }
}

impl Attributes for VecAttributes {
    #[inline]
    fn is_empty(&self) -> bool {
        self.attributes.is_empty()
    }
}

impl Display for VecAttributes {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.write_str(&self.build_str())
    }
}

impl Add for VecAttributes {
    type Output = VecAttributes;

    fn add(mut self, mut other: VecAttributes) -> VecAttributes {
        for (k, v) in other.attributes.drain(..) {
            VecAttributes::add(&mut self, k, v);
        }
        self
    }
}

#[derive(Clone, Debug)]
pub struct CachedAttributes {
    attributes: Arc<String>,
}

impl CachedAttributes {
    #[inline]
    pub fn new() -> Self {
        CachedAttributes {
            attributes: Arc::new(String::new()),
        }
    }

    #[inline]
    pub fn with_attributes(attributes: String) -> Self {
        CachedAttributes {
            attributes: Arc::new(attributes),
        }
    }
}

impl Attributes for CachedAttributes {
    #[inline]
    fn is_empty(&self) -> bool {
        self.attributes.is_empty()
    }
}

impl Display for CachedAttributes {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.write_str(&self.attributes)
    }
}

impl From<VecAttributes> for CachedAttributes {
    fn from(attributes: VecAttributes) -> Self {
        CachedAttributes::with_attributes(attributes.build_str())
    }
}

impl<'a> Add<&'a CachedAttributes> for VecAttributes {
    type Output = CombinedAttributes<'a>;

    fn add(self, other: &'a CachedAttributes) -> CombinedAttributes<'a> {
        CombinedAttributes::with_attributes(self, other)
    }
}

impl<'a> Add<VecAttributes> for &'a CachedAttributes {
    type Output = CombinedAttributes<'a>;

    fn add(self, other: VecAttributes) -> CombinedAttributes<'a> {
        CombinedAttributes::with_attributes(other, self)
    }
}
