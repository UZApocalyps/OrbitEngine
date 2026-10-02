# OrbitEngine

Moteur de visual novel en Rust (wgpu + winit), avec un éditeur no-code à venir. Cibles : Windows, Linux, Web (WebGL2), Android plus tard. Le plan complet est dans [`docs/ROADMAP.md`](docs/ROADMAP.md), les décisions d'architecture dans [`docs/adr/`](docs/adr).

État : jalon **M1 terminé** (renderer 2D, texte, audio, VM, UI minimale). Le player joue une histoire RON (fond, personnages, choix, musique, transitions, ~30 répliques) en natif et dans le navigateur, avec rechargement à chaud en natif.

## Structure

| Chemin | Rôle |
|---|---|
| `crates/orbit_core` | IR de l'histoire (IDs ULID, `Value`, RON, validation, migrations). Sans GPU, compile en wasm. |
| `crates/orbit_project` | API de commandes unique (undo/redo, transactions, événements, versions par nœud). Sans GPU. |
| `crates/orbit_vm` | VM de l'histoire : `step()` pur et non bloquant, événements de présentation, état de scène déclaratif, snapshots sérialisables. Sans GPU/audio, compile en wasm. |
| `crates/orbit_cli` | Binaire `orbit` : `validate`, `run-text` (jeu en terminal), `mcp` (placeholder M3). |
| `crates/orbit_platform` | Fenêtre et boucle d'application (winit), natif et web. |
| `crates/orbit_gfx` | Couche wgpu : surface, textures RGBA8, pipeline de sprites (WebGL2 inclus). |
| `crates/orbit_render2d` | Renderer 2D : sprites batchés, calques, caméra virtuelle 1920x1080 + letterbox, render targets, transitions (fade, dissolve, masque). |
| `crates/orbit_text` | Texte (`cosmic-text`) : atlas de glyphes, machine à écrire, balises `{color=}`, `{b}`, `{i}`, `{w=}`. |
| `crates/orbit_audio` | Audio (`kira`) : musique avec boucle et fade, effets, bus de volume, déblocage par geste utilisateur. |
| `crates/orbit_assets` | Chargement asynchrone (threads en natif, `fetch` sur le web), décodage PNG/WAV/OGG, surveillance de fichier (hot reload). |
| `crates/orbit_ui` | UI en jeu data-driven (`Theme` RON) : textbox, écran de choix, écrans de chargement/fin/erreur. |
| `crates/orbit_runtime` | Runtime : orchestre VM, assets, rendu, texte, audio et UI (chargement, « cliquer pour démarrer », transitions, hot reload). Indépendant du binaire. |
| `crates/orbit_player` | Binaire du jeu (natif et wasm) : lit les arguments / l'URL et lance `orbit_runtime`. Contient aussi la démo visuelle codée en dur (`--demo`). |
| `xtask` | Automatisation : `web`, `serve`, `check-arch`, `gen-assets`. |
| `examples/stories/demo.ron` | Histoire d'exemple. |
| `assets/demo/`, `assets/fonts/` | Assets de la démo (générés par `cargo xtask gen-assets`) et police Inter (OFL). |

## Commandes

Prérequis : Rust via `rustup` (la toolchain 1.94 et la cible wasm sont installées par `rust-toolchain.toml`).

```sh
cargo run -p orbit_player          # joue examples/stories/demo.ron (natif)
cargo run -p orbit_player -- ma/story.ron --root ma/    # une autre histoire
cargo run -p orbit_player -- --demo                     # démo visuelle codée en dur
cargo run -p orbit_cli -- validate examples/stories/demo.ron --check-files
cargo test --workspace             # tests
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo deny check                   # licences, bans, sources, advisories

# Web
cargo install wasm-bindgen-cli --version 0.2.129 --locked   # doit égaler la version de Cargo.lock
cargo xtask web                    # build wasm32 release + bundle (assets, polices, histoires d'exemple) dans target/web/
cargo xtask serve                  # http://127.0.0.1:8080 (Ctrl+C pour arrêter) ; --port <n> pour un autre port
# puis ouvrir / (démo M1), /?story=examples/stories/demo.ron ou /?demo (démo visuelle)

# Architecture : core/project sans GPU ni fenêtre, compilables en wasm
cargo xtask check-arch
```

## Jouer une histoire

* **Contrôles** : clic, Espace ou Entrée pour avancer (le premier appui complète la machine à écrire, le suivant passe à la réplique ; pendant une transition, il la termine immédiatement). Choix : survol et clic souris, flèches haut/bas + Entrée, ou touches 1 à 9.
* **Natif** : `orbit_player [histoire.ron | dossier-projet] [--root <dossier>] [--no-watch]`. La racine des assets (chemins du manifest) est `--root`, sinon le dossier du projet, sinon le dossier courant (comme `orbit validate --check-files`) ; la démo par défaut utilise la racine du dépôt.
* **Hot reload** (natif) : si le fichier RON change sur disque, il est rechargé, revalidé et l'histoire redémarre du début ; une erreur s'affiche à l'écran jusqu'à correction.
* **Web** : `?story=<chemin>` relatif à la page (le bundle contient `examples/stories/` et `assets/`).
* Au lancement, un écran « Cliquer pour démarrer » débloque l'audio (obligatoire dans les navigateurs, conservé en natif).

`RUST_LOG=debug cargo run -p orbit_player` règle le niveau de logs natif ; sur le web les logs vont dans la console du navigateur.

## Licence

Provisoire : `MIT OR Apache-2.0` (voir ROADMAP §6). Seules des dépendances sous licences permissives sont acceptées (`deny.toml`).
