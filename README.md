# SQLX Leak Issue Minimal Reproduction Project

This project serves as a minimal reproducible test case for
https://github.com/launchbadge/sqlx/issues/83.

## Test Setup

MacBook Pro (16-inch, 2019) running macOS Catalina (10.15.2, build 19C57)
with a 2.3GHz 8-core i9 and 32GB of RAM. Rust toolchain: stable 1.40.0.
`wrk` v4.1.0 installed via homebrew. Postgres 10 (I've been lazy about upgrading)
is running as my test database via Postgres.app Version 2.3.2 (63).

## Repro Instructions

Bear with me... this can take a bit of work.

### Optional sqlx patch

I found it helpful to use a locally patched sqlx build with one modification.
In `queue.rs`, I modified the final match of the `pop` function as follows:

```rust
// Timed out waiting for a connection
// Error is not forwarded as its useless context
Err(_) => {
    println!("Timout info: {} waiting; {:?} connections; {} idle", self.waiters.len(), size.current(), self.idle.len());
    Err(Error::PoolTimedOut(None))
},
```

This may provide you with a bit of insight as to what's happening under the hood.

### Running the server

1. Set up your `DATABASE_URL` env var.
1. Run server using `cargo run --release`.

### Repro instructions

1. In another terminal, run `wrk -t20 -c500 -d30s "http://localhost:8000/add?x=1&y=2"`.
1. Open up Postman (or another terminal and `curl` if you like) and ping it with a
similar request during the test.
1. Killing `wrk` with `Ctrl-C` tends to break things faster.
1. Keep on doing this loop until things break. (You could probably make the pool size
smaller to make it fail faster...)

Eventually, you will have leaked all connections, but the pool will not realize this,
so every connection will result in an acquire timeout. If you added the extra log as
above, you will see it pop up as connections time out. In this example, I have made
the timeout unreasonably low compared to real-world use so that it's easier
to demonstrate the issue, but in reality, I was able to break it using a single
query with an average runtime of 20ms (single run with no load) and a connection
timeout of 30s, so it never timed out. This is intentionally contrived because I
found that it made things fail faster.

You can monitor your "progress" by checking the number of connections to your
postgres server via your favorite method. I use `SELECT * FROM pg_stat_activity`.
You will see the number of active connections drop as they are "leaked" and they
will not be replaced by new connections.

## Misc notes

* I simulated a long-running query by doing *two* `SELECT`s and a database level
sleep. My real-world use case was a non-trivial, but not exactly long-running
PostGIS query that took ~20ms under light load and ~150ms under extreme load on my
MacBook.
* The pool continues to time out on all `acquire` operations even after the connection
life is exceeded.
* When some condition occurs, new connections will be initiated (watch the server for
log messages). I have not determined exactly what causes this, but it is not always
a leak indicator, as you may see dozens of these without an actual "leak."

## Other issue discovered while testing

I left in a commented `test_on_acquire` line in `main.rs` because of some odd
behavior I noticed while troubleshooting the pool. In my "real" code, which
only did a single operation with the connection, if I disable `test_on_acquire`,
I will randomly start receiving responses to other (presumably leaked) queries
instead of the one I submitted. I have been unable to leak with just a simple
addition query, but a non-trivial query with easily verifiable arugments
will produce this result. I run my tests using different query args
for `wrk` and Postman to detect this condition.

The issue does not seem to appear as long as `test_on_acquire` is `true`.
In the simpler example in this repo that uses `pg_sleep`, you will actually
get a slew of errors that may help to pinpoint the source of the issue.

### Reproduction steps

1. Configure the pool `test_on_acquire` to `false`.
1. Start a server as above.
1. Do a simple query with Postman or w/e to verify that it works.
1. In another terminal, run `wrk` as above and wait for it to complete.
If you have the extra logging modification, you will see a few hundred
waiting operations most likely.
1. After `wrk` finishes, run a request in Postman.

You should end up with something like this in your console. I have inclueded
one of the extra log lines for reference.

```
Timout info: 476 waiting; 10 connections; 0 idle
thread 'tokio-runtime-worker' panicked at 'assertion failed: 8 <= buf.len()', /Users/ianthetechie/.cargo/registry/src/github.com-1ecc6299db9ec823/byteorder-1.3.2/src/lib.rs:1970:9
stack backtrace:
   0: <std::sys_common::backtrace::_print::DisplayBacktrace as core::fmt::Display>::fmt
   1: core::fmt::write
   2: std::io::Write::write_fmt
   3: std::panicking::default_hook::{{closure}}
   4: std::panicking::default_hook
   5: std::panicking::rust_panic_with_hook
   6: std::panicking::continue_panic_fmt
   7: rust_begin_unwind
   8: core::panicking::panic_fmt
   9: core::panicking::panic
  10: sqlx_core::postgres::types::float::<impl sqlx_core::decode::Decode<sqlx_core::postgres::database::Postgres> for f64>::decode
  11: <sqlx_core::postgres::row::PgRow as sqlx_core::row::Row>::get
  12: sqlx_leak_repro::route::{{closure}}
  13: hyper::proto::h1::dispatch::Dispatcher<D,Bs,I,T>::poll_catch
  14: <hyper::server::conn::upgrades::UpgradeableConnection<I,S,E> as core::future::future::Future>::poll
  15: <hyper::common::drain::Watching<F,FN> as core::future::future::Future>::poll
  16: <hyper::server::conn::spawn_all::NewSvcTask<I,N,S,E,W> as core::future::future::Future>::poll
  17: tokio::task::core::Core<T>::poll
  18: std::panicking::try::do_call
  19: __rust_maybe_catch_panic
  20: tokio::task::harness::Harness<T,S>::poll
  21: tokio::runtime::thread_pool::worker::GenerationGuard::run_task
  22: tokio::runtime::thread_pool::worker::GenerationGuard::run
  23: std::thread::local::LocalKey<T>::with
  24: tokio::runtime::thread_pool::worker::Worker::run
  25: tokio::task::core::Core<T>::poll
  26: std::panicking::try::do_call
  27: __rust_maybe_catch_panic
  28: tokio::task::harness::Harness<T,S>::poll
  29: tokio::runtime::blocking::pool::Inner::run
  30: tokio::runtime::context::enter
note: Some details are omitted, run with `RUST_BACKTRACE=full` for a verbose backtrace.
```
