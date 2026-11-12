# Plugin store: catalogs, sources, signatures

The editor's plugin manager (**Tools > Extensions > Plugins…**) has three tabs:

* **Installed**: the plugins in `<config>/plugins/` (install from a file, enable /
  disable, uninstall, restart), unchanged from 0.4.0, plus an update badge when a
  catalog has a newer build for this editor;
* **Store**: every plugin of the enabled catalog sources, with search, source and
  tag filters, install / update / installed state and details (changelog, license,
  homepage);
* **Sources**: the catalog sources (add by URL, enable / disable, remove, refresh,
  trust level).

Code: `crates/orbit_sign` (SHA-256, SHA-512, Ed25519, no dependency),
`crates/orbit_http` (minimal HTTP client, `curl` for https, `file://`),
`crates/orbit_plugin_store` (catalog format, sources, trust, cache, download and
verification, update detection), `crates/orbit_editor/src/builtin/plugins.rs`
and `builtin/plugin_store.rs` (UI), `xtask/src/plugin_store.rs`
(`plugin-key`, `--sign`, `plugin-index`).

## 1. Catalog format (`index.json`, schema 1)

A source is one JSON file. Unknown fields are ignored (forward compatible); a
`schema_version` greater than the one the editor knows is refused with "this
catalog needs a newer editor".

```json
{
  "schema_version": 1,
  "source": {
    "name": "OrbitEngine official plugins",
    "homepage": "https://github.com/UZApocalyps/OrbitEnginePlugins",
    "description": { "en": "Plugins published by the OrbitEngine team.", "fr": "Plugins publiés par l'équipe OrbitEngine." }
  },
  "generated": "2026-11-12T10:00:00Z",
  "plugins": [
    {
      "id": "orbit.ai_studio",
      "name": "AI Studio",
      "description": { "en": "AI game-creation wizard…", "fr": "Assistant de création de jeu par IA…" },
      "author": "OrbitEngine contributors",
      "homepage": "https://github.com/UZApocalyps/OrbitEnginePlugins",
      "license": "MIT OR Apache-2.0",
      "icon": "https://raw.githubusercontent.com/UZApocalyps/OrbitEnginePlugins/main/plugins/orbit.ai_studio/icon.png",
      "tags": ["ai", "writing"],
      "versions": [
        {
          "version": "0.4.0",
          "released": "2026-11-12",
          "changelog": { "en": "First store release.", "fr": "Première version du store." },
          "builds": [
            {
              "editor_version": "0.4.0",
              "fingerprint": "3f1c0e9a7b2d4c5e8f9a0b1c2d3e4f50",
              "target": "x86_64-pc-windows-msvc",
              "profile": "release",
              "url": "https://github.com/UZApocalyps/OrbitEnginePlugins/releases/download/orbit.ai_studio-v0.4.0/orbit.ai_studio-0.4.0-windows-x86_64.orbitplugin",
              "sha256": "<64 lowercase hex>",
              "size": 1234567,
              "signature": "<128 lowercase hex, or null>"
            }
          ]
        }
      ]
    }
  ]
}
```

| Field | Rule |
|---|---|
| `schema_version` | integer, required; `1` today |
| `source.name` | required, shown on the badges |
| `plugins[].id` | lowercase dotted id (`acme.hello`), unique in the file; `orbit.*` is reserved to the official source |
| `description`, `changelog` | either a string or `{ "en": …, "fr": … }` (the interface language, then `en`, then any) |
| `tags` | lowercase words, used by the filters |
| `versions[].version` | `major.minor.patch[-pre]`, unique per plugin |
| `builds[].fingerprint` | the **editor build id** (below): a build is offered only to an editor whose id is equal |
| `builds[].url` | `https://`, `http://` or `file://`, or a path relative to the `index.json` URL |
| `builds[].sha256` / `size` | of the `.orbitplugin` file; checked before installing |
| `builds[].signature` | Ed25519 signature (hex) of the build message (below), or `null` |

An entry that breaks a rule is skipped (with a warning in the Sources tab); the
rest of the catalog stays usable.

### Editor build id (`fingerprint`)

A native plugin only loads in the exact editor build it was compiled for (see
`orbit_editor_api::native`). The build id is the first 16 bytes (32 lowercase hex
characters) of the SHA-256 of the fingerprint key
`api_version|abi|rustc|egui|target|profile|api_hash|layout_hash|type_hash`
(`EditorFacts::cache_key`, `orbit_plugin_store::fingerprint_id`). The Store tab
shows the editor's; `cargo xtask plugins` computes it from the plugin's own
fingerprint (read by `orbit-editor --probe-plugin`).

The store picks, for each plugin, the **newest version that has a build whose id
equals the editor's**. Otherwise the card says "no build for your editor version"
and lists the editor versions that have one.

## 2. Signatures

Ed25519 (RFC 8032) over this UTF-8 message (each line ends with `\n`):

```
orbit-plugin-build/v1
id=<plugin id>
version=<plugin version>
fingerprint=<editor build id>
sha256=<sha256 of the .orbitplugin, lowercase hex>
```

Keys and signatures are lowercase hex (public key 64 characters, signature 128).
The private key file holds the 32-byte seed in hex (`cargo xtask plugin-key gen`).

## 3. Sources and trust

`<config>/plugin_store/sources.ron` lists the sources; the official one is always
present (it can be disabled, not removed, and its URL and key cannot be changed).

| Trust | When | Builds accepted |
|---|---|---|
| **Official** | the built-in source `https://raw.githubusercontent.com/UZApocalyps/OrbitEnginePlugins/main/index.json` | only with a valid signature by the official key embedded in the editor (`orbit_plugin_store::OFFICIAL_PUBLIC_KEY`) |
| **Trusted** | a source added with a public key | only with a valid signature by that key |
| **Unsigned** | a source added without a key | signature ignored, SHA-256 and size still checked; red warning before each install |

Rules that hold whatever the trust:

* a build is downloaded to `<config>/plugin_store/downloads/`, its size and
  SHA-256 checked, then its signature, and only then handed to the installer,
  which runs the usual checks (file format, descriptor in a child process,
  fingerprint, id, editor range) before replacing anything;
* updates of an installed plugin are only taken from the source it was installed
  from (`<config>/plugin_store/installs.ron`); a plugin installed from a file only
  gets updates from the official source. A third-party catalog listing the same id
  with a higher version never "updates" it silently: installing from it shows a
  "replaces the plugin installed from …" warning;
* ids starting with `orbit.` are only accepted from the official source.

Adding a source accepts a GitHub repository URL (`https://github.com/owner/repo`
resolves to `https://raw.githubusercontent.com/owner/repo/main/index.json`;
`/tree/<branch>` and `/blob/<branch>/<path>` are understood), a direct `index.json`
URL, or a `file://` URL / local path.

## 4. Cache and network

Catalogs are cached in `<config>/plugin_store/cache/<source>.json` with their
`ETag` / `Last-Modified`; a refresh sends `If-None-Match` / `If-Modified-Since`
(304: the cache is used). When the network fails the cached catalog is shown with
an "offline, cached on …" note. The Store tab refreshes catalogs older than one
hour when it opens; the Refresh button forces it.

HTTP: plain `http://` and `file://` in-process; `https://` through the system
`curl` (Windows 10+, macOS, Linux), so the editor carries no TLS stack. Downloads
run as editor tasks (progress, cancel in the status bar).

## 5. Releasing an official plugin

1. Once: `cargo xtask plugin-key gen` writes the private key **outside any
   repository** (default `../OrbitEngine-secrets/plugin_signing_key` next to the
   repository; refused inside a git work tree) and prints the public key. Put the
   file's content in the GitHub secret `ORBIT_PLUGIN_SIGNING_KEY` of
   `UZApocalyps/OrbitEnginePlugins`; the public key goes in
   `orbit_plugin_store::OFFICIAL_PUBLIC_KEY` (an editor release).
2. In `OrbitEnginePlugins`, run the **Release plugins** workflow with the editor
   tag (or push a tag `editor-<editor tag>`, e.g. `editor-v0.5.0`; the editor tag
   must contain this tooling, so 0.4.0 cannot be used): it checks out OrbitEngine at the editor tag, copies
   `plugins/*` of the plugins repository into the engine workspace, builds the
   editor and the plugins with the engine's toolchain and `Cargo.lock`
   (Windows and Linux), runs `cargo xtask plugins --sign-env ORBIT_PLUGIN_SIGNING_KEY`,
   uploads the `.orbitplugin` files to a GitHub release
   `plugins-<editor tag>-r<run>` and commits the updated `index.json`
   (`cargo xtask plugin-index`); the Pages site redeploys.
3. The editor sees the new version at the next catalog refresh.

Build ids only match when the editor users run was built exactly like the CI
build (same rustc, `Cargo.lock`, target, release profile): build the editor
releases in CI from the same tag, and pin an exact rustc (`1.95.0`, not `1.95`) in
`rust-toolchain.toml`.

Manually:

```sh
cargo build -p orbit_editor --release
cargo xtask plugins --sign ../OrbitEngine-secrets/plugin_signing_key
cargo xtask plugin-index --index ../OrbitEnginePlugins/index.json --builds target/plugins \
  --url "https://github.com/UZApocalyps/OrbitEnginePlugins/releases/download/{tag}/{file}" \
  --tag plugins-v0.5.0-r1
```

`cargo xtask plugins` writes, next to each package, `<package>.orbitplugin.json`
(id, name, description, author, homepage, version, editor version, build id,
target, profile, file, size, sha256, signature) that `plugin-index` merges into
`index.json` (a build with the same id, version and build id is replaced).

## 6. Third-party catalogs

Anyone can host a catalog: a repository with an `index.json` in this format and
its `.orbitplugin` files (GitHub releases or any HTTPS host). Generate your own key
with `cargo xtask plugin-key gen --out <file>`, sign with `--sign <file>`, and
publish the public key so users can add your source as **Trusted**.
