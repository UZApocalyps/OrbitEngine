# Plugin store: catalogs, sources, signatures

The editor's plugin manager (**Tools > Extensions > Plugins…**) has three tabs:

* **Installed**: the plugins in `<config>/plugins/` (install from a file, enable /
  disable, uninstall, restart), unchanged from 0.4.0, plus an update badge when a
  catalog has a newer build for this editor;
* **Store**: every plugin of the enabled catalog sources, with search, source and
  tag filters, install / update / installed state and details (changelog, license,
  homepage); its **Game plugins (project)** switch lists the game plugins, which
  install as sources into the open project (§7);
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
          "version": "0.5.0",
          "released": "2026-11-12",
          "changelog": { "en": "First store release.", "fr": "Première version du store." },
          "builds": [
            {
              "editor_version": "0.5.0",
              "fingerprint": "3f1c0e9a7b2d4c5e8f9a0b1c2d3e4f50",
              "target": "x86_64-pc-windows-msvc",
              "profile": "release",
              "url": "https://github.com/UZApocalyps/OrbitEnginePlugins/releases/download/orbit.ai_studio-v0.5.0/orbit.ai_studio-0.5.0-windows-x86_64.orbitplugin",
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

### 5.1 Plugins shipped with the editor

The editor distributions (installers, portable archive) can ship plugin packages in
`bundled-plugins/` next to the executable (`cargo xtask plugins --bundle [ids]`, default
`orbit.renpy_import` and `orbit.mcp`, which used to be built into the editor). They are
unrelated to the catalogs: the editor installs them at start-up through the same code as an
install from a file (no network, nothing relaxed), once per id (`bundled_offered` in
`plugins.ron`, so an uninstalled one does not return), and updates only the plugins that came
from such a package. They have no store origin: the catalogs' update check does not apply to them. Any later install
over one (from a file or the store) makes it the user's, no longer updated from `bundled-plugins/`.

## 6. Third-party catalogs

Anyone can host a catalog: a repository with an `index.json` in this format and
its `.orbitplugin` files (GitHub releases or any HTTPS host). Generate your own key
with `cargo xtask plugin-key gen --out <file>`, sign with `--sign <file>`, and
publish the public key so users can add your source as **Trusted**.

## 7. Game plugins (sources, per project)

Game plugins (runtime plugins, `orbit_plugin_api::RuntimePlugin`) are Rust crates
linked statically into the player at export (`orbit_export::plugins`). The store
distributes them **as sources**, installs them **in the open project**
(`plugins/<dir>/`, never in the editor's plugin folder) and lists them in the
**Store** tab under **Game plugins (project)** with a **Game** badge; the
**Installed** tab has a "Game plugins of <project>" section.

Code: `crates/orbit_plugin_store/src/game/` (`catalog`, `package`, `download`,
`project`), `crates/orbit_editor/src/builtin/game_plugins.rs` (UI),
`xtask/src/game_plugin.rs` (`cargo xtask game-plugin pack | index`).

### 7.1 Catalog: `game_plugins` (still schema 1)

Game plugins are a separate, optional top-level array of `index.json`. It is not a
"kind" field on `plugins[]` entries: an editor that predates it ignores unknown
fields, so it does not see game plugins at all (instead of listing source packages
as editor plugins with "no build for your editor"), and an old `plugin-index`
keeps working on the editor plugins. `schema_version` stays `1` (raising it would
make older editors refuse the whole catalog). A catalog without `game_plugins` is
unchanged; a broken `game_plugins` is ignored with a warning and never makes the
editor plugins of the catalog unusable.

```json
"game_plugins": [
  {
    "id": "orbit.dice",
    "name": "Dice",
    "description": { "en": "…", "fr": "…" },
    "author": "", "homepage": null, "license": "MIT OR Apache-2.0", "icon": null,
    "tags": ["dice"],
    "versions": [
      {
        "version": "0.5.0",
        "released": "2026-11-12",
        "changelog": "First store release.",
        "api_version": 1,
        "min_engine": "0.5.0",
        "max_engine": "0.6.0",
        "url": "https://…/orbit.dice-0.5.0.orbitsrc",
        "sha256": "<64 lowercase hex>",
        "size": 4889,
        "signature": "<128 lowercase hex, or null>"
      }
    ]
  }
]
```

| Field | Rule |
|---|---|
| `id`, `name`, `description`, `tags`, `url` | as for editor plugins (§1); `orbit.*` only from the official source |
| `versions[].api_version` | the `orbit_plugin_api::API_VERSION` the crate is written against |
| `min_engine` / `max_engine` | engine versions it builds with: `min <= engine < max` (`max` optional) |
| `sha256` / `size` | of the `.orbitsrc` file, at most 16 MB |

The store offers, for each game plugin, the **newest version whose `api_version`
equals the editor's and whose engine range contains the editor version** (no build
id: sources are compiled with the engine at export).

### 7.2 Package: `.orbitsrc`

A zip holding the crate files at its root and `orbit-game-plugin.json`:

```json
{
  "format": 1,
  "id": "orbit.dice", "plugin_ids": ["orbit.dice"],
  "name": "Dice", "version": "0.5.0", "description": "…", "author": "", "license": "…",
  "crate_name": "orbit-dice", "dir": "orbit-dice",
  "api_version": 1, "min_engine": "0.5.0", "max_engine": "0.6.0",
  "engine_crates": ["orbit_core", "orbit_plugin_api", "orbit_vm"],
  "build_script": false, "proc_macro": false,
  "files": { "Cargo.toml": "<sha256>", "src/lib.rs": "<sha256>" }
}
```

Reading a package checks everything **before writing anything**: package at most
16 MB, each file 16 MB, 64 MB unpacked (a zip bomb stops at the limit), 2000 files;
every entry path relative, `/`-separated, without `..`, `.`, empty parts, drive or
root, backslash, `:`, control characters, trailing dot or space, or Windows device
name (`con`, `nul`, `aux`…); no symbolic link or special entry; no duplicate up to
case; no path that is both a file and a folder; the files are exactly those of
`files`, each with its SHA-256; `Cargo.toml` at the root; `.orbit-store.json` and
the manifest name are reserved. The package SHA-256 is what the signature covers,
so a valid signature vouches for the manifest and every file.

`Cargo.toml` is read by a small line reader (no TOML library in the editor). It
follows multi-line arrays and inline tables, quoted (`"path"`) and dotted
(`foo.path`, `dependencies.foo.path`) keys and `[target.'cfg(…)'.dependencies]`,
and **refuses**, at packing and again at install, every form it cannot read for
sure: multi-line strings, unbalanced brackets, `[patch]` / `[replace]` /
`[workspace]` tables, `package.workspace`, `workspace = true`, escapes in a path,
a dependency path that is not `…/crates/orbit_*`, a crate file path (`[lib] path`,
`build`) leaving the crate, and a multi-line `[package.metadata.orbit] plugins`.
At install the crate name, version and plugin ids of `Cargo.toml` must equal the
manifest's (the exporter reads `Cargo.toml`), and every engine crate it uses must
exist in the engine sources.

### 7.3 Signatures and trust

Same trust levels as editor plugins (§3: Official, Trusted, Unsigned with the red
warning), over a different message, so an editor build signature is never valid for
a game package:

```
orbit-game-plugin/v1
id=<plugin id>
version=<plugin version>
api_version=<n>
sha256=<sha256 of the .orbitsrc, lowercase hex>
```

The installer then checks the package against the catalog (id, version, SHA-256),
refuses `orbit.*` plugin ids from a non-official source, and checks the package's
own `api_version` and engine range. A game plugin is code that **runs on the
developer's machine when the game is exported** (build scripts and procedural
macros run at compile time; the package says when it has some) and in the players'
game: every install from a non-official source asks first, with that warning.

### 7.4 Installing in a project

* The package is downloaded to `<config>/plugin_store/downloads/`, size, SHA-256
  and signature checked, then extracted into `.orbit/plugin-staging/` of the
  project and moved to `plugins/<dir>/` by a rename; the previous folder is only
  removed once the new one is in place (any failure puts it back).
* `plugins/<dir>/.orbit-store.json` records the store id, plugin ids, version,
  source (id, name, URL, trust), package SHA-256, API version, engine range, the
  engine folder used and the SHA-256 of every file **as installed**. It travels with
  the project (version control): updates only come from that source (matched by id,
  or by URL on another machine), never from another catalog.
* Engine dependencies: plugin crates depend on the engine crates by path (the
  exporter needs the same `orbit_plugin_api`). The installer points every path
  dependency `…/crates/orbit_*` at the engine sources the exporter uses
  (`ORBIT_ENGINE_SRC`, else the tree the editor was built from). When none is
  found the crate installs as packaged and the UI says the export will fail until
  it is reinstalled with `ORBIT_ENGINE_SRC` set. The packager refuses other path
  dependencies and workspace inheritance (`version.workspace = true`…), and drops
  a `[lints] workspace = true` table.
* `orbit.project.ron` > `plugins` gets `{ id, version }` for each plugin id of the
  crate (removed on uninstall, ids dropped by an update removed), with the same
  load / save as the export profiles: installing files is not an undoable story
  edit. A bare story folder (no manifest) installs without it.
* **Local changes are never overwritten silently.** Installing or updating is
  refused when the target folder exists without a record (a crate written by
  hand), when another crate of the project provides the same plugin id, or when
  the installed files differ from the record (`target/` and `Cargo.lock` are
  ignored). The UI lists the changed files and offers **Replace (keep a backup)**:
  the folder is moved to `.orbit/plugin-backups/<dir>-<time>/`, nothing is deleted.
  Uninstalling deletes a pristine folder (the store can install it again) and moves
  a changed one to the backups.

### 7.5 The editor half, the toolchain

The editor only knows the node types of a plugin whose editor half is loaded
(`orbit_plugin_api::Plugin::register`, i.e. an editor plugin with the same id, from
the Store's editor plugins or built in). The Installed section says, per plugin id:
loaded (steps editable), "its editor plugin is in the Store", or "no editor part"
(steps stay read-only in the editor, as for a missing plugin, and the exported game
runs them). It also checks `cargo --version` and the engine sources once per
project: missing ones do not block the install, the UI says the export will fail.

### 7.6 Releasing a game plugin

```sh
cargo xtask game-plugin pack examples/plugins/dice --sign ../OrbitEngine-secrets/plugin_signing_key
cargo xtask game-plugin index --index ../OrbitEnginePlugins/index.json \
  --url "https://github.com/UZApocalyps/OrbitEnginePlugins/releases/download/{tag}/{file}" --tag game-v0.5.0
```

`pack` reads `Cargo.toml` (`[package]` name, version, description, license;
`[package.metadata.orbit]` `plugins` and an optional display `name`), skips
`target/`, `Cargo.lock` and hidden files, records this engine's `API_VERSION`, and
sets the engine range to this version up to the next minor (excluded) unless
`--min-engine` / `--max-engine <v>|none` say otherwise. It writes
`<target>/game-plugins/<id>-<version>.orbitsrc` and `<file>.json` (the descriptor).
`index` merges the descriptors into `game_plugins` (a version listed again is
replaced) and keeps the editor plugins; `plugin-index` keeps `game_plugins` too.
Unsigned descriptors are refused unless `--allow-unsigned`.
