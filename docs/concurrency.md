# Nimiq Rust implementation concurrency design

This document describes the concurrency architecture for the Nimiq Rust implementation, both the library and the implementation. Please take into account that this document is more of a work in progress draft than a finalized design, and as such it will be updated along with the development.

## Basic concepts

There are basically two orthogonal ways of running different processes (as in "things", not as in operating system processes) in a non-blocking way: using async code and running code in a parallel manner (this requires a multi-core CPU).

Async code means that while a process is waiting for stuff to be done, it will yield control of the core to another process. The initial process can be scheduled to  run again after the condition it is waiting for is fullfilled.

Running code in parallel (as its name implies) means using multiple resources (i.e. cores) in a CPU at the same time.


## Rust concurrency primitives

[Mio](https://github.com/carllerche/mio) is a low level library that basically just wraps  the OS async notification facilities (i.e. `epoll` in Linux, `kqueue` in *BSD, etc.) in a portable and Rust idiomatic way. This means that you have to manually register which events you want to be notified about for each I/O resource and then poll them manually and react to their respective notifications (in other words, you need to provide your own event loop and scheduler when working with `mio`).

[Futures](https://github.com/rust-lang-nursery/futures-rs) is basically a library providing a set of abstractions around the notion of a value (`future`) or values (`stream`) that may not be available immediately but in the future (hence the name). This library also provides some utils to work with `futures`.

[Tokio](https://tokio.rs/) is a framework that unites the primitives from `mio` and `futures` into another abstraction layer to make it easier to write async protocols (seems geared toward networking more than other use cases). Tokio seems to be split into different parts:

  * tokio-io: this part provides abstractions for working with I/O.
  * tokio-runtime: provides an *out of the box* reactor (event loop), executor (task-stealing thread pool) and timer support to simplify development and reduce boilerplate code in applications.
  * tokio-core: FIXME

**Warning**: these libraries/crates seem to be under heavy development right now, so there are changes all over the place (f.e. [`async/await` syntax](https://boats.gitlab.io/blog/post/2018-04-06-async-await-final/), and a [new API on `futures 0.3`](https://rust-lang-nursery.github.io/futures-rs/blog/2018/07/19/futures-0.3.0-alpha.1.html)) and some stuff is being deprecated (f.e. [`tokio-core`](https://github.com/tokio-rs/tokio-core#deprecation-notice)) and some functionality provided right now in one of the crates will be moved into another one (f.e. [the `tokio-executor` will](https://github.com/tokio-rs/tokio/issues/211#issuecomment-375534010) be moved [to the `futures` crate](https://www.reddit.com/r/rust/comments/7syxw4/rust_2018_core_embeddedsimd_intermesiate/dtaefoj/)).

## Nimiq Rust client design

Ideally the Rust implementation should be divided in two parts: a library that could be embedded on other applications and the proper application that would use the API provided by the library itself (currently, the node.js implementation has a design similar to this, but with everything bundled in one repository and with a mostly integrated build process).

Taking this into account, at the concurrency level, it would make sense to provide all the `futures`/`tokio` interfaces (i.e. traits) at the library level and then it would be the application's responsility to provide the reactor (event loop) and executor (thread pool) to run the whole thing (this seems to be the recommended way of implementing libraries, plus it gives the most flexibility so that the application can integrate other stuff into the event loop, such as a GUI, instead of having different event loops that could/would block each other).

## Links to remember
https://www.reddit.com/r/rust/comments/8x0mkc/how_does_mio_and_tokio_work/
https://www.reddit.com/r/rust/comments/7klghl/tokio_internals_understanding_rusts_asynchronous/
https://cafbit.com/post/tokio_internals/
https://users.rust-lang.org/t/returning-server-handle-to-consumers-tokio/19105
https://www.reddit.com/r/rust/comments/8qi0cx/advice_on_choosing_between_mio_and_tokio/