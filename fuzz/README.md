How to fuzz
===========

Setup
-----

See also the official documentation:
https://rust-fuzz.github.io/book/afl/setup.html.

First, you're going to need to clone this repository and install `cargo-afl`:
```sh
git clone https://github.com/nimiq/core-rs-albatross
cargo install cargo-afl
```


Running
-------

See also the official documentation:
https://rust-fuzz.github.io/book/afl/tutorial.html.

```sh
cd fuzz
cargo afl build --features fuzz --release
cargo afl fuzz -i in/key_nibbles -o out/key_nibbles ../target/release/key_nibbles
```

You can replace `key_nibbles` with other files in the [`src/bin`](src/bin)
directory, at the time of writing there are `bitset`, `key_nibbles` and
`trie_node`.

Note that as of the time of writing (2024-10-21), the existing test cases have
already been ran long enough that it is unlikely to turn up any new bugs.


Adding new things to fuzz
-------------------------

Copy [`src/bin/key_nibbles.rs`](src/bin/key_nibbles.rs) to a new name and make
the relevant adjustments. Add the creation of seeds to
[`src/bin/00_testcases.rs`](src/bin/00_testcases). Run `cargo run
--bin=00_testcases`. Then fuzz like described in the previous section.
