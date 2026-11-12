//! Catalogs over the network, cached in `<config>/plugin_store/cache/`:
//! `<source>.json` (the last good catalog as received) and
//! `<source>.meta.ron` (URL, `ETag`, `Last-Modified`, when fetched). A
//! refresh is conditional (`If-None-Match` / `If-Modified-Since`, 304 keeps
//! the cache); when it fails the cached catalog is used and the error kept.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::catalog::{CatalogError, ParsedCatalog, parse};
use crate::sources::SourceEntry;
use crate::{StorePaths, now_secs, write_atomic};

/// Catalogs older than this are fetched again when the store opens.
pub const MAX_AGE: Duration = Duration::from_secs(3600);
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheMeta {
    pub url: String,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
    /// When the server last confirmed or sent it (seconds since 1970).
    pub fetched_at: u64,
}

/// A source's catalog as the store shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceCatalog {
    pub parsed: ParsedCatalog,
    pub fetched_at: u64,
    /// The last refresh failed: this is the cached copy (the error).
    pub offline: Option<String>,
}

fn cache_paths(
    paths: &StorePaths,
    source: &SourceEntry,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = paths.cache_dir();
    (
        dir.join(format!("{}.json", source.id)),
        dir.join(format!("{}.meta.ron", source.id)),
    )
}

fn read_meta(path: &std::path::Path) -> Option<CacheMeta> {
    ron::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn write_meta(path: &std::path::Path, meta: &CacheMeta) {
    if let Ok(text) = ron::ser::to_string_pretty(meta, ron::ser::PrettyConfig::default()) {
        let _ = write_atomic(path, text.as_bytes());
    }
}

/// The cached catalog of `source`, if any (no network).
pub fn cached(paths: &StorePaths, source: &SourceEntry) -> Option<SourceCatalog> {
    let (body, meta) = cache_paths(paths, source);
    let meta = read_meta(&meta).filter(|m| m.url == source.url)?;
    let text = std::fs::read_to_string(body).ok()?;
    let parsed = parse(&text, source.builtin).ok()?;
    Some(SourceCatalog {
        parsed,
        fetched_at: meta.fetched_at,
        offline: None,
    })
}

/// Fetches the catalog of `source`: the cache when it is younger than
/// `max_age` (unless `force`), else a conditional request; on failure the
/// cache (with the error in `offline`), or the error when nothing is cached.
pub fn fetch(
    paths: &StorePaths,
    source: &SourceEntry,
    force: bool,
    max_age: Duration,
) -> Result<SourceCatalog, String> {
    let (body_path, meta_path) = cache_paths(paths, source);
    let meta = read_meta(&meta_path).filter(|m| m.url == source.url);
    let cache = cached(paths, source);
    let now = now_secs();
    if !force
        && let (Some(c), Some(m)) = (&cache, &meta)
        && now.saturating_sub(m.fetched_at) < max_age.as_secs()
    {
        return Ok(c.clone());
    }
    let fallback = |error: String| match &cache {
        Some(c) => Ok(SourceCatalog {
            offline: Some(error),
            ..c.clone()
        }),
        None => Err(error),
    };
    let mut request =
        orbit_http::Request::get(&source.url, TIMEOUT).header("Accept", "application/json");
    if let (Some(m), Some(_)) = (&meta, &cache) {
        if let Some(etag) = &m.etag {
            request = request.header("If-None-Match", etag.clone());
        }
        if let Some(lm) = &m.last_modified {
            request = request.header("If-Modified-Since", lm.clone());
        }
    }
    let response = match request.send() {
        Ok(r) => r,
        Err(e) => return fallback(e),
    };
    match response.status {
        304 => match (cache, meta) {
            (Some(c), Some(mut m)) => {
                m.fetched_at = now;
                write_meta(&meta_path, &m);
                Ok(SourceCatalog {
                    fetched_at: now,
                    ..c
                })
            }
            _ => Err("HTTP 304 without a cached catalog".into()),
        },
        200..=299 => {
            let text = match String::from_utf8(response.body.clone()) {
                Ok(t) => t,
                Err(_) => return fallback(CatalogError::Invalid("not UTF-8".into()).to_string()),
            };
            let parsed = match parse(&text, source.builtin) {
                Ok(p) => p,
                Err(e) => return fallback(e.to_string()),
            };
            if write_atomic(&body_path, text.as_bytes()).is_ok() {
                write_meta(
                    &meta_path,
                    &CacheMeta {
                        url: source.url.clone(),
                        etag: response.header("etag").map(str::to_owned),
                        last_modified: response.header("last-modified").map(str::to_owned),
                        fetched_at: now,
                    },
                );
            }
            Ok(SourceCatalog {
                parsed,
                fetched_at: now,
                offline: None,
            })
        }
        status => fallback(format!("HTTP {status}")),
    }
}

/// Forgets the cache of a removed source.
pub fn forget(paths: &StorePaths, source: &SourceEntry) {
    let (body, meta) = cache_paths(paths, source);
    let _ = std::fs::remove_file(body);
    let _ = std::fs::remove_file(meta);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::Sources;
    use crate::test_util::scratch;
    use orbit_http::testing::{Answer, Server};

    const CATALOG: &str = r#"{"schema_version":1,"source":{"name":"Mock"},"plugins":[{"id":"acme.hello","name":"Hello"}]}"#;

    fn source(url: &str) -> SourceEntry {
        let mut s = Sources::default();
        s.add(url, "Mock", "").unwrap()
    }

    #[test]
    fn catalogs_are_cached_revalidated_and_used_offline() {
        let dir = scratch("fetch");
        let paths = StorePaths { root: dir.clone() };
        // 304 when the client sends the ETag it got.
        let server = Server::start_with(Some(Box::new(|req| {
            (req.path == "/index.json" && req.header("if-none-match") == Some("\"v1\""))
                .then(|| Answer::status(304))
        })));
        server.route(
            "/index.json",
            Answer::ok(CATALOG).with_header("ETag", "\"v1\""),
        );
        let src = source(&server.url("/index.json"));

        assert!(cached(&paths, &src).is_none());
        let first = fetch(&paths, &src, false, MAX_AGE).unwrap();
        assert_eq!(first.parsed.catalog.plugins.len(), 1);
        assert_eq!(first.offline, None);
        // Fresh: no request.
        let n = server.seen().len();
        fetch(&paths, &src, false, MAX_AGE).unwrap();
        assert_eq!(server.seen().len(), n);
        // Forced: conditional request, 304, the cache.
        let again = fetch(&paths, &src, true, MAX_AGE).unwrap();
        assert_eq!(
            server.seen().last().unwrap().header("if-none-match"),
            Some("\"v1\"")
        );
        assert_eq!(again.parsed, first.parsed);
        assert!(cached(&paths, &src).is_some());

        // The server breaks: the cache, with the error.
        server.route("/index.json", Answer::status(500));
        let mut no_etag = src.clone();
        no_etag.id = src.id.clone();
        std::fs::write(
            dir.join("cache").join(format!("{}.meta.ron", src.id)),
            format!("(url: \"{}\", fetched_at: 0)", src.url),
        )
        .unwrap();
        let offline = fetch(&paths, &no_etag, false, MAX_AGE).unwrap();
        assert_eq!(offline.offline.as_deref(), Some("HTTP 500"));
        assert_eq!(offline.parsed, first.parsed);

        // A broken catalog does not replace the good cache.
        server.route("/index.json", Answer::ok("{not json"));
        let kept = fetch(&paths, &src, true, MAX_AGE).unwrap();
        assert!(kept.offline.as_deref().unwrap().contains("invalid catalog"));
        assert_eq!(kept.parsed, first.parsed);

        // Unreachable and nothing cached: an error.
        let nowhere = source("http://127.0.0.1:9/index.json");
        assert!(fetch(&paths, &nowhere, true, MAX_AGE).is_err());
        forget(&paths, &src);
        assert!(cached(&paths, &src).is_none());
    }

    #[test]
    fn file_sources_work_without_network() {
        let dir = scratch("fetch_file");
        std::fs::write(dir.join("index.json"), CATALOG).unwrap();
        let paths = StorePaths {
            root: dir.join("store"),
        };
        let src = source(&dir.to_string_lossy());
        assert!(src.url.starts_with("file:///"));
        let c = fetch(&paths, &src, true, MAX_AGE).unwrap();
        assert_eq!(c.parsed.catalog.source.name, "Mock");
    }
}
