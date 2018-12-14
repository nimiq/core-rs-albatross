extern crate gcc;

fn main() {
//    let target = std::env::var("TARGET").unwrap();

    let mut config = gcc::Build::new();
    config.file("native/argon2.c")
        .file("native/core.c")
        .file("native/blake2/blake2b.c")
        .file("native/opt.c");

    config.opt_level(2);

    config.include("native");
    config.flag("-DARGON2_NO_THREADS");

    config.compile("libargon2.a");
}
