fn main() {
    let version = if let Ok(git_commit) = build_data::get_git_commit() {
        format!("g{}", &git_commit[..16])
    } else {
        format!("v{}", env!("CARGO_PKG_VERSION"))
    };
    println!("cargo::rustc-env=NIMIQ_POW_MIGRATION_VERSION={version}");
    println!("cargo::rerun-if-changed=../.git/index");
    println!("cargo::rerun-if-changed=../.git/logs/HEAD");
}
