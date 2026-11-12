//! The minimal blocking HTTP client (plain `http://` over std, `https://`
//! through the system `curl`, no TLS stack) now lives in `orbit_http`, shared
//! with the plugin store.

pub use orbit_http::{Request, Response, encode_query};
