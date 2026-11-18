# What uni-term changed

xattr 0.2.3 from crates.io (MIT or Apache-2.0), used by gpui 0.2.2 through gpui_http_client and zed-async-tar.

One change, in `src/sys/mod.rs`: on Linux and Android `ENOATTR` is `libc::ENODATA`. libc 0.2.190 and later no longer define `ENOATTR` there, so the crate did not build on Linux.

Remove this folder and the `[patch.crates-io]` entry in the root `Cargo.toml` when gpui no longer needs xattr 0.2.
