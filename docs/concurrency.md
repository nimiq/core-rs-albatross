## Rust concurrency primitives

[Mio](https://github.com/carllerche/mio) is a low level library that basically just wraps  the OS notification facilities (i.e. `epoll` in Linux, `kqueue` in *BSD, etc.) for non-blocking operations in a portable and Rust idiomatic way.

This means that you have to manually register which events you want to be notified about for each I/O resource (you use tokens for this), then poll to know which tokens have had notifications already and react to them in some way (in other words, you need to provide your own event loop and scheduler when working with `mio`).

This is a very simple code example (taken from [`mio`'s documentation](https://docs.rs/mio/0.6.15/mio/)) of how this works:

```rust
use mio::*;
use mio::net::{TcpListener, TcpStream};

// Setup some tokens to allow us to identify which event is
// for which socket.
const SERVER: Token = Token(0);
const CLIENT: Token = Token(1);

let addr = "127.0.0.1:13265".parse().unwrap();

// Setup the server socket
let server = TcpListener::bind(&addr).unwrap();

// Create a poll instance
let poll = Poll::new().unwrap();

// Start listening for incoming connections
poll.register(&server, SERVER, Ready::readable(),
              PollOpt::edge()).unwrap();

// Setup the client socket
let sock = TcpStream::connect(&addr).unwrap();

// Register the socket
poll.register(&sock, CLIENT, Ready::readable(),
              PollOpt::edge()).unwrap();

// Create storage for events
let mut events = Events::with_capacity(1024);

loop {
    poll.poll(&mut events, None).unwrap();

    for event in events.iter() {
        match event.token() {
            SERVER => {
                // Accept and drop the socket immediately, this will close
                // the socket and notify the client of the EOF.
                let _ = server.accept();
            }
            CLIENT => {
                // The server just shuts down the socket, let's just exit
                // from our event loop.
                return;
            }
            _ => unreachable!(),
        }
    }
}
```

[Futures](https://github.com/rust-lang-nursery/futures-rs) is basically a library providing a set of abstractions (e.g. `Traits`) and utils (e.g. [`combinators`](https://tokio.rs/docs/going-deeper/futures-mechanics/)) around the notion of a value (`future`) or values (`stream`) that may not be available immediately but could be in the future (hence the name).

[This tutorial series](https://dev.to/mindflavor/rust-futures-an-uneducated-short-and-hopefully-not-boring-tutorial---part-1-3k3) is a very good place to get a more in-depth knowledge about futures (you also get to construct a future from scratch there, which helps a lot to better understand the concepts).

Also, see the note below about current development in `futures`, since a lot of stuff is changing right now for version `0.3`.

[Tokio](https://tokio.rs/) is a framework that unites the primitives from `mio` and `futures` into another abstraction layer to make it easier to write async stuff. Tokio seems to be split into different parts:

  * tokio-io: this part provides primitive futures for working with network I/O.
  * tokio-runtime: provides an *out of the box* reactor (event loop), executor (task-stealing thread pool) and timer support to simplify development and reduce boilerplate code in applications.

  [There is](https://rust-lang-nursery.github.io/futures-rs/blog/2018/08/17/toykio.html) a "toy" implementation of tokio, which was developed so that people could more easily understand how tokio works.

**Warning**: these libraries/crates seem to be under heavy development right now, so there are changes all over the place (f.e. [`async/await` syntax](https://boats.gitlab.io/blog/post/2018-04-06-async-await-final/), and a [new API on `futures 0.3`](https://rust-lang-nursery.github.io/futures-rs/blog/2018/07/19/futures-0.3.0-alpha.1.html)) and some stuff is being deprecated (f.e. [`tokio-core`](https://github.com/tokio-rs/tokio-core#deprecation-notice)) and some functionality provided right now in one of the crates will be moved into another one (f.e. [the `tokio-executor` will](https://github.com/tokio-rs/tokio/issues/211#issuecomment-375534010) be moved [to the `futures` crate](https://www.reddit.com/r/rust/comments/7syxw4/rust_2018_core_embeddedsimd_intermesiate/dtaefoj/)).

## Nimiq Rust client design

Ideally the Rust implementation should be divided in two parts: a library that could be embedded on other applications and the proper application that would use the API provided by the library itself (currently, the node.js implementation has a design similar to this, but with everything bundled in one repository and with a mostly integrated build process).

Taking this into account, at the concurrency level, it would make sense to provide all the `futures`/`tokio` interfaces (i.e. traits) at the library level and then it would be the application's responsility to provide the reactor (event loop) and executor (thread pool) to run the whole thing (this seems to be the recommended way of implementing libraries, plus it gives the most flexibility so that the application can integrate other stuff into the event loop, such as a GUI, instead of having different event loops that could/would block each other).

## Links to remember
 * https://www.reddit.com/r/rust/comments/8x0mkc/how_does_mio_and_tokio_work/
 * https://www.reddit.com/r/rust/comments/7klghl/tokio_internals_understanding_rusts_asynchronous/
 * https://cafbit.com/post/tokio_internals/
 * https://users.rust-lang.org/t/returning-server-handle-to-consumers-tokio/19105
 * https://www.reddit.com/r/rust/comments/8qi0cx/advice_on_choosing_between_mio_and_tokio/
 * https://users.rust-lang.org/t/how-to-create-reuseable-client-with-tokio-tcpstream/18058/6
 * https://aturon.github.io/apr/async-in-rust/chapter.html
 * https://news.ycombinator.com/item?id=12029238
