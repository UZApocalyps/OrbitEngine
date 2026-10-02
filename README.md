# OrbitEngine

Moteur de visual novel en Rust (wgpu + winit) avec son éditeur no-code (en construction). Cibles du jeu : Windows, Linux, Web (WebGL2), Android plus tard ; l'éditeur est natif (Windows et Linux). Le plan complet est dans [`docs/ROADMAP.md`](docs/ROADMAP.md), les décisions d'architecture dans [`docs/adr/`](docs/adr).

État : jalon **M3b terminé** (preview intégrée : le vrai runtime joue l'histoire ouverte dans un panneau de l'éditeur, avec rechargement à chaud depuis la mémoire et « jouer depuis ici ») après le **M3a** (socle de l'éditeur : fenêtre egui sur le device wgpu partagé, outils intégrés sous forme de plugins, palette de commandes, projets, undo / redo, sauvegarde atomique, récupération après crash, rendu à la demande) et le **M2** (fonctionnalités VN de base : menu principal, pause, rollback, skip/auto, historique, saves multi-slots avec miniatures et autosave, préférences, i18n, variables persistantes). Le player joue une histoire RON (fond, personnages, choix, musique, transitions) en natif et dans le navigateur, avec rechargement à chaud en natif. Prochain jalon : M3c (éditeur de scène linéaire, personnages, inspecteur auto-généré ; architecture dans l'[ADR 0009](docs/adr/0009-architecture-editeur.md)).

## Structure

| Chemin | Rôle |
|---|---|
| `crates/orbit_core` | IR de l'histoire (IDs ULID, `Value`, RON, validation, migrations). Sans GPU, compile en wasm. |
| `crates/orbit_project` | API de commandes unique (undo/redo, transactions, événements, versions par nœud). Sans GPU. |
| `crates/orbit_vm` | VM de l'histoire : `step()` pur et non bloquant, événements de présentation, état de scène déclaratif, snapshots, rollback / roll-forward, backlog, variables persistantes, recherche de chemin bornée vers un nœud (`seek`, pour « jouer depuis ici »). Sans GPU/audio, compile en wasm. |
| `crates/orbit_save` | Saves (format versionné `postcard`, compatibilité après édition de l'histoire), slots manuels / autosaves en rotation / quicksave, données persistantes et préférences (RON), stockage fichiers (natif) ou `localStorage` (web). Sans GPU, compile en wasm. |
| `crates/orbit_i18n` | Localisation : traductions RON par `StringId`, repli sur la langue source, couverture. Sans GPU, compile en wasm. |
| `crates/orbit_plugin_api` | API de plugins, côté projet : manifeste, types de nœuds personnalisés, validateurs, importeurs, outils MCP, hooks d'événements, `API_VERSION`. Sans GPU ni egui, compile en wasm (contrôlée par `check-arch`). |
| `crates/orbit_editor_api` | API de plugins, côté éditeur (egui) : `EditorPlugin`, `EditorContext` (lecture du projet, commandes seulement, sélection), registres de panneaux, actions (palette, menus, raccourcis) et inspecteurs. |
| `crates/orbit_editor` | L'éditeur (binaire `orbit-editor`, natif) : `EditorCore` (sans GPU, testé en headless), dock `egui_dock`, thème maison clair / sombre, palette de commandes, projets, récupération, les outils intégrés qui sont tous des plugins, et `PreviewHost` (le runtime embarqué, sa render target enregistrée dans egui). Jamais lié au player. |
| `crates/orbit_cli` | Binaire `orbit` : `validate`, `run-text` (jeu en terminal), `mcp` (placeholder M3). |
| `crates/orbit_platform` | Fenêtre et boucle d'application (winit), natif et web ; requêtes à l'hôte (plein écran, quitter), horloge, fuseau et langue du système. |
| `crates/orbit_gfx` | Couche wgpu : surface, textures RGBA8, pipeline de sprites (WebGL2 inclus). |
| `crates/orbit_render2d` | Renderer 2D : sprites batchés, calques, caméra virtuelle 1920x1080 + letterbox, render targets (rendu dans une cible, y compris les transitions), relecture asynchrone d'une render target (miniatures, capture de la preview). |
| `crates/orbit_text` | Texte (`cosmic-text`) : atlas de glyphes, machine à écrire, balises `{color=}`, `{b}`, `{i}`, `{w=}`. |
| `crates/orbit_audio` | Audio (`kira`) : musique avec boucle et fade, effets, bus de volume, déblocage par geste utilisateur. |
| `crates/orbit_assets` | Chargement asynchrone (threads en natif, `fetch` sur le web), décodage PNG/WAV/OGG, surveillance de fichier (hot reload). |
| `crates/orbit_ui` | UI en jeu data-driven (`Theme` RON) : textbox, choix, widgets, écrans (menu principal, pause, sauvegarde/chargement, préférences, historique, quick menu, toasts), textes d'interface en/fr. |
| `crates/orbit_runtime` | Runtime : orchestre VM, assets, rendu, texte, audio et UI. `Shell` = machine d'état sans GPU (écrans, saves, autosave, préférences, langue, persistants), `Game` = skip/auto, `Director` = VM + rollback, `Runtime` = GPU, audio, miniatures ; en mode « embarqué » (`Runtime::new_embedded`) il charge une `Story` depuis la mémoire, joue dans une render target et se recharge à chaud (module `preview` : points de départ et reprise, sans GPU). Indépendant du binaire. |
| `crates/orbit_player` | Binaire du jeu (natif et wasm) : lit les arguments / l'URL et lance `orbit_runtime`. Contient aussi la démo visuelle codée en dur (`--demo`) et la galerie des écrans d'UI avec de fausses données (`--ui-gallery`). |
| `xtask` | Automatisation : `web`, `serve`, `check-arch`, `gen-assets`. |
| `tools/make_ui_fonts.py` | Génère les copies d'Inter de l'éditeur (`assets/fonts/ui/`) sans les glyphes d'usage privé qui masquent les icônes Phosphor. |
| `examples/stories/demo.ron` | Histoire d'exemple ; `examples/stories/i18n/` : sa traduction française et `index.txt` (liste des traductions). |
| `assets/demo/`, `assets/fonts/` | Assets de la démo (générés par `cargo xtask gen-assets`) et police Inter (OFL). |

## Commandes

Prérequis : Rust via `rustup` (la toolchain 1.95 et la cible wasm sont installées par `rust-toolchain.toml`). Linux : les bibliothèques de développement X11 / Wayland / xkbcommon / Vulkan (voir `.github/workflows/ci.yml`).

```sh
cargo run -p orbit_editor                               # l'éditeur (accueil, projets récents)
cargo run -p orbit_editor -- examples/stories/demo.ron  # ouvre l'histoire de démo (ou un dossier de projet)
cargo run -p orbit_player          # joue examples/stories/demo.ron (natif)
cargo run -p orbit_player -- ma/story.ron --root ma/    # une autre histoire
cargo run -p orbit_player -- --new-game                 # sans menu principal, nouvelle partie directe
cargo run -p orbit_player -- --data-dir saves/          # saves et préférences dans un dossier choisi
cargo run -p orbit_player -- --demo                     # démo visuelle codée en dur
cargo run -p orbit_player -- --ui-gallery               # tous les écrans d'UI, données factices
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

## L'éditeur

`cargo run -p orbit_editor [-- <projet | histoire.ron>]` (ajoutez `--release` pour la vitesse ; `orbit-editor --help` liste les options).

* **Projet** : un dossier avec `orbit.project.ron` (nom, fichier d'histoire, plugins requis), `story.ron` (l'histoire, RON canonique, lisible par le player et `orbit validate`), `assets/` et `.orbit/` (état d'outil : récupération après crash, ignoré par git). Fichier > Nouveau projet crée un projet minimal ; Fichier > Ouvrir un projet / une histoire (`.ron` seul, sans dossier de projet) utilise le sélecteur de fichiers natif ; la page d'accueil liste les projets récents.
* **Interface** : dock (explorateur à gauche, accueil au centre, inspecteur à droite, console en bas ; onglets déplaçables, disposition conservée dans `%APPDATA%\OrbitEngine\Editor` ou `~/.config/orbit-engine/editor`), thème sombre ou clair (Affichage), menus et **palette de commandes** (Ctrl+K ou Ctrl+Maj+P : recherche floue, raccourcis affichés ; Affichage > « Afficher : … » rouvre un panneau fermé).
* **Édition (M3a)** : sélectionner une réplique dans l'explorateur, modifier son texte et son personnage dans l'inspecteur, ajouter (Ctrl+Entrée) ou supprimer (Suppr) une réplique ; l'éditeur de scène complet est M3c. Tout passe par les commandes d'`orbit_project` : Ctrl+Z / Ctrl+Y (ou Ctrl+Maj+Z) annulent et rétablissent (une saisie de texte = une étape), Ctrl+S enregistre de façon atomique, le titre et la barre d'état signalent les modifications non enregistrées, la fermeture demande confirmation. Un enregistrement automatique de secours est écrit toutes les 30 s tant que le projet est modifié ; après un arrêt brutal, le lancement suivant propose de restaurer.
* **Preview (M3b)** : le panneau Preview (en haut à droite par défaut ; menu Preview, palette) joue le **vrai runtime** dans une texture d'egui sur le même device wgpu, au format 16:9. Barre d'outils : lecture / pause (F6), arrêt (Maj+F6), recommencer (Ctrl+F6), **jouer depuis ici** (F7, bouton « Ici », ou la cible sur la ligne sélectionnée / le menu contextuel de l'explorateur : l'état réel au nœud — décor, personnages, musique, variables — est reconstitué par une recherche de chemin bornée dans les choix ; si aucun chemin n'existe, repli sur l'état par défaut avec un avertissement dans la console), vitesse 1× / 2× / saut (s'arrête aux choix), son coupé, suivi de la lecture dans l'explorateur, plein panneau. Cliquez dans l'image pour lui donner le focus : alors seulement la souris et le clavier (Entrée, Espace, flèches, 1-9, Échap, H, S, A, F5, F9, Ctrl) vont au jeu ; un clic ailleurs rend le clavier à l'éditeur. **Chaque modification de l'histoire** (frappe, annulation, ajout) recharge la preview depuis l'état mémoire (environ 100 ms de délai anti-rebond, puis < 5 ms) et reprend au même nœud ; si le nœud a disparu, au label le plus proche avant lui, sinon au début ; une histoire invalide n'est pas jouée (la dernière version valide continue, l'erreur est lisible dans le panneau). Les saves de la preview restent en mémoire. La preview ne dessine que pendant la lecture active (frappe du texte, transitions, pauses, saut) : en pause, arrêtée, masquée, fenêtre en arrière-plan ou jeu qui attend un clic, **0 frame**. Options de debug : `--play-preview`, `--preview-skip`, `--play-from-selection` (avec `--select-node`), `--edit-text <n>=<texte>` (simule une frappe), `--preview-capture <png>` (relecture de la cible, 16:9), `--always-active` (ignore la perte de focus de la fenêtre).
* **Plugins** : chaque outil (explorateur, inspecteur, console, accueil, preview) est un `EditorPlugin` ; le « Plugin d'essai » (Affichage > Afficher : Plugin d'essai, ou Outils) montre un panneau, des actions et un type de nœud ajoutés par l'API publique.
* **Performance** : l'éditeur ne dessine une frame que sur événement ou échéance (0 frame au repos). `--frame-counter` journalise chaque frame, `--screenshot <png> --exit-after <s>` enregistre la frame rendue (utile aux tests visuels : `--open-palette`, `--theme light`, `--select-node <n>`, `--run-action <id>`).

## Jouer une histoire

* **Écrans** : « Cliquer pour démarrer » (débloque l'audio, obligatoire dans les navigateurs, conservé en natif) puis menu principal (Nouvelle partie, Continuer = la save la plus récente, Charger, Préférences, Quitter en natif). En jeu, Échap ou clic droit ouvre la pause (Reprendre, Sauvegarder, Charger, Historique, Préférences, Menu principal) ; Échap / clic droit ferme l'écran ouvert. À la fin de l'histoire, un clic revient au menu principal.
* **Avancer** : clic, Espace ou Entrée (le premier appui complète la machine à écrire, le suivant passe à la réplique ; pendant une transition, il la termine). Choix : survol et clic, flèches haut/bas + Entrée, ou touches 1 à 9. Les clics sur le quick menu (Retour, Historique, Saut, Auto, Q.Save, Q.Load, Préf., Menu) n'avancent pas le dialogue.
* **Rollback** : molette vers le haut, flèche gauche, Page précédente ou bouton « Retour » reviennent d'une réplique (scène, personnages et musique sont restaurés sans transition) ; molette vers le bas, flèche droite ou Page suivante avancent dans l'historique conservé. « Revenir ici » dans l'historique (touche H) saute à une réplique précise.
* **Skip / Auto** : Ctrl maintenu = saut temporaire ; bouton ou touche S = bascule ; bouton ou touche A = mode auto. Le saut s'arrête aux choix et, en mode « texte lu uniquement » (préférence), aux répliques non lues ; l'auto attend la fin de la frappe puis un délai proportionnel à la longueur. Toute action du joueur coupe skip et auto.
* **Saves** : F5 sauvegarde rapide, F9 chargement rapide, écrans Sauvegarder / Charger (emplacements manuels, autosaves, sauvegarde rapide ; miniature, date, scène, dernière réplique). Autosave au début de chaque scène et toutes les 20 répliques, plus une dernière autosave à la fermeture de la fenêtre. Une save devenue incompatible est grisée ; une save endommagée donne une notification, jamais un crash. Emplacement : natif `%APPDATA%\OrbitEngine\<jeu>` (Windows), `~/Library/Application Support/OrbitEngine/<jeu>` (macOS), `$XDG_DATA_HOME` ou `~/.local/share/OrbitEngine/<jeu>` (Linux), ou `--data-dir` ; web : `localStorage` (clés `orbit/<jeu>/...`, environ 5 Mo par origine).
* **Préférences** (écran dédié, enregistrées à chaque changement) : volumes des bus, vitesse du texte, délai auto, mode skip, plein écran (natif ; F11 ou Alt+Entrée), langue.
* **Langues** : le player charge les traductions listées dans `i18n/index.txt` à côté de l'histoire (fichiers `<histoire>.<langue>.ron`, voir `examples/stories/i18n/`) ; sans préférence, la langue du système ou du navigateur est utilisée si elle est disponible, sinon la langue source. Changement à chaud : la réplique affichée est traduite sur place.
* **Natif** : `orbit_player [histoire.ron | dossier-projet] [--root <dossier>] [--no-watch] [--new-game] [--data-dir <dossier>]`. La racine des assets (chemins du manifest) est `--root`, sinon le dossier du projet, sinon le dossier courant (comme `orbit validate --check-files`) ; la démo par défaut utilise la racine du dépôt.
* **Hot reload** (natif) : si le fichier RON change sur disque, il est rechargé, revalidé et l'histoire redémarre du début ; une erreur s'affiche à l'écran jusqu'à correction.
* **Web** : `?story=<chemin>` relatif à la page (le bundle contient `examples/stories/` et `assets/`) ; `?new-game` démarre directement une partie après le clic initial.

`RUST_LOG=debug cargo run -p orbit_player` règle le niveau de logs natif ; sur le web les logs vont dans la console du navigateur.

## Licence

Provisoire : `MIT OR Apache-2.0` (voir ROADMAP §6). Seules des dépendances sous licences permissives sont acceptées (`deny.toml`).
