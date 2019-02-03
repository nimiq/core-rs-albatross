pub const CODE: u32 = 1;

pub fn is_compatible(code: u32) -> bool {
    // Allow future, backwards-compatible versions.
    code >= CODE
}
