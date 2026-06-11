use nimiq_primitives::policy::Policy;

pub fn for_each_protocol_version(mut f: impl FnMut(u16)) {
    for version in 0..=Policy::max_supported_version() {
        f(version);
    }
}
