fn main() {
    let version = env!("CARGO_PKG_VERSION");

    // When building from a work tree with uncommitted changes, append a
    // `+dirty` SemVer build-metadata identifier (e.g. `1.6.0+dirty`). Build
    // metadata is ignored for version precedence, so the dirty build still
    // compares equal to the released version it is based on.
    //
    // `build_data::get_git_dirty()` returns `Err` when Git is unavailable or we
    // are not inside a work tree (e.g. building from a published crate or a
    // source tarball), in which case no dirty marker is appended.
    let version = match build_data::get_git_dirty() {
        Ok(true) => format!("{version}+dirty"),
        Ok(false) | Err(_) => version.to_string(),
    };
    println!("cargo::rustc-env=NIMIQ_VERSION={version}");

    // Re-run the build script when the checked-out commit or branch changes so
    // the marker stays in sync. Best-effort: ignore the error when not inside a
    // Git work tree.
    let _ = build_data::rerun_if_git_commit_or_branch_changed();
}
