use std::cell::Cell;
use std::time::Duration;

use super::testing::{Answer, Server};
use super::*;

const T: Duration = Duration::from_secs(5);

fn scratch(name: &str) -> PathBuf {
    // Under the target folder (never the system temp drive).
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../target/test-tmp/orbit_http_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn plain_http_get_with_headers_redirects_and_chunks() {
    let server = Server::start();
    server.route("/a", Answer::ok("hello").with_header("ETag", "\"v1\""));
    server.route("/moved", Answer::status(302).with_header("Location", "/a"));
    server.route("/chunked", Answer::ok(vec![7u8; 2500]).chunked());

    let r = Request::get(&server.url("/a"), T)
        .header("If-None-Match", "\"v0\"")
        .send()
        .unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(r.text(), "hello");
    assert_eq!(r.header("etag"), Some("\"v1\""));
    assert_eq!(server.seen()[0].header("if-none-match"), Some("\"v0\""));

    let r = Request::get(&server.url("/moved"), T).send().unwrap();
    assert_eq!((r.status, r.text().as_str()), (200, "hello"));

    let r = Request::get(&server.url("/chunked"), T).send().unwrap();
    assert_eq!(r.body, vec![7u8; 2500]);

    assert_eq!(
        Request::get(&server.url("/missing"), T)
            .send()
            .unwrap()
            .status,
        404
    );
    assert!(Request::get("ftp://x/y", T).send().is_err());
}

#[test]
fn downloads_stream_with_progress_limit_and_cancel() {
    let dir = scratch("download");
    let server = Server::start();
    let data: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
    server.route("/big", Answer::ok(data.clone()));
    server.route("/big_chunked", Answer::ok(data.clone()).chunked());

    for path in ["/big", "/big_chunked"] {
        let dest = dir.join("out.bin");
        let calls = Cell::new(0);
        let done = Request::get(&server.url(path), T)
            .download(&dest, 1 << 20, &|got, _| {
                calls.set(calls.get() + 1);
                got <= 200_000
            })
            .unwrap();
        assert_eq!((done.status, done.size), (200, 200_000));
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert!(calls.get() > 0);
    }
    // Beyond the limit.
    let err = Request::get(&server.url("/big"), T)
        .download(&dir.join("x"), 1000, &|_, _| true)
        .unwrap_err();
    assert_eq!(err, DownloadError::TooLarge);
    // Cancelled.
    let err = Request::get(&server.url("/big"), T)
        .download(&dir.join("y"), 1 << 20, &|_, _| false)
        .unwrap_err();
    assert_eq!(err, DownloadError::Cancelled);
}

#[test]
fn file_urls_read_and_copy_local_files() {
    let dir = scratch("file");
    let src = dir.join("with space.json");
    std::fs::write(&src, b"{}").unwrap();
    let url = file_url(&src);
    assert!(url.starts_with("file:///"), "{url}");
    assert!(url.contains("%20"));
    assert_eq!(
        file_url_path(&url).unwrap(),
        PathBuf::from(src.to_string_lossy().replace('\\', "/"))
    );
    let r = Request::get(&url, T).send().unwrap();
    assert_eq!((r.status, r.body.as_slice()), (200, &b"{}"[..]));
    let done = Request::get(&url, T)
        .download(&dir.join("copy"), 10, &|_, _| true)
        .unwrap();
    assert_eq!((done.status, done.size), (200, 2));
    let missing = file_url(&dir.join("nope.json"));
    assert_eq!(Request::get(&missing, T).send().unwrap().status, 404);
}

#[test]
fn urls_resolve_against_a_base() {
    let base = "https://example.org/repo/main/index.json";
    assert_eq!(
        resolve_url(base, "builds/a.orbitplugin"),
        "https://example.org/repo/main/builds/a.orbitplugin"
    );
    assert_eq!(resolve_url(base, "./a"), "https://example.org/repo/main/a");
    assert_eq!(resolve_url(base, "../x/a"), "https://example.org/repo/x/a");
    assert_eq!(resolve_url(base, "/root"), "https://example.org/root");
    assert_eq!(resolve_url(base, "https://other/x"), "https://other/x");
    assert_eq!(resolve_url("http://h:1/index.json", "a"), "http://h:1/a");
    assert_eq!(
        resolve_url("file:///E:/cat/index.json", "p/a.zip"),
        "file:///E:/cat/p/a.zip"
    );
}

#[test]
fn curl_header_blocks_skip_redirects_and_continues() {
    let out = b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 302 Found\r\nLocation: x\r\n\r\nHTTP/2 200\r\netag: \"e\"\r\n\r\nBODY";
    let (status, headers, consumed) = last_head(out).unwrap();
    assert_eq!(status, 200);
    assert_eq!(header_in(&headers, "etag"), Some("\"e\""));
    assert_eq!(&out[consumed..], b"BODY");
}
