# Premiers pas

## Installer l'éditeur

Téléchargez l'installeur de votre système depuis la page des Releases du dépôt.

| Système | Fichier | Installation |
|---|---|---|
| Windows | `orbit-editor_<version>_x64-setup.exe` (NSIS) | Double-clic ; par utilisateur (sans droits administrateur) ou pour tous. Mode silencieux : `/S`. |
| Windows | `orbit-editor_<version>_x64_en-US.msi` (WiX) | Double-clic ou `msiexec /i fichier.msi` (installation par machine, droits administrateur). |
| Linux | `.deb` | `sudo apt install ./orbit-editor_<version>_amd64.deb` |
| Linux | `.AppImage` | `chmod +x` puis lancer le fichier (aucune installation). |

Windows : l'installeur crée un raccourci dans le menu Démarrer (et sur le Bureau) et une entrée de désinstallation dans « Applications et fonctionnalités ». Les installeurs ne sont pas encore signés : SmartScreen peut demander une confirmation. Il n'y a pas d'auto-update : installez la nouvelle version par-dessus l'ancienne.

La version de l'éditeur est affichée en bas à droite de la barre d'état.

### Contenu installé

À côté de l'exécutable `orbit-editor` :

* `examples/stories/` et `assets/demo/` : le projet de démonstration (ouvrez `examples/stories/demo.ron`) ;
* `templates/` : les **templates de player** utilisés par l'export de jeux (Windows, Linux, web). Le dossier peut être vide tant que les templates ne sont pas livrés avec l'installeur ; l'éditeur indique alors lequel manque.

Les polices et le logo sont intégrés à l'exécutable : l'éditeur ne dépend d'aucun chemin du dépôt.

### Construire les installeurs soi-même

```sh
cargo install cargo-packager --locked
cargo build --release -p orbit_editor
cd crates/orbit_editor
cargo packager --release --formats wix,nsis        # Windows
cargo packager --release --formats appimage,deb    # Linux
```

Les installeurs sont écrits dans `target/packager/`. WiX et NSIS sont téléchargés automatiquement au premier build Windows. La configuration est dans `crates/orbit_editor/Cargo.toml` (`[package.metadata.packager]`). La CI les produit sur un tag `v*` (ou à la demande) et les attache à une Release GitHub en brouillon.

## Créer un projet

1. Lancez l'éditeur : l'écran d'accueil propose **Nouveau projet…** (Ctrl+N). Choisissez un dossier et un nom : l'éditeur crée `orbit.project.ron` (le manifeste : nom, histoire, profils d'export), `story.ron` (l'histoire) et le dossier des assets.
2. Pour essayer d'abord, ouvrez le projet de démonstration livré avec l'éditeur (`examples/stories/demo.ron`).

Un projet est un dossier ; tous les chemins d'assets de l'histoire sont relatifs à lui :

```text
mon_jeu/
  orbit.project.ron   manifeste (nom, histoire, profils d'export)
  story.ron           l'histoire
  i18n/               traductions : story.<langue>.ron
  assets/             images, musiques, sons
```

## Éditer

* **Scène** (éditeur linéaire) et **Graphe** : écrivez les répliques, les choix, les sauts de scène. **Inspecteur** : tous les réglages d'un nœud. **Personnages** et **Variables** : les définitions.
* **Bibliothèque d'assets** : glissez-déposez vos images (PNG, JPEG) et sons (WAV, OGG) dans la fenêtre ; ils sont copiés dans `assets/` et déclarés.
* **Problèmes** (zone du bas) : la validation en direct. Une erreur bloque l'export, un avertissement non.
* Ctrl+S enregistre ; Ctrl+Z / Ctrl+Y annulent / rétablissent. Une IA peut éditer avec vous : voir [mcp.md](mcp.md).

## Prévisualiser

Le panneau **Preview** joue l'histoire avec le vrai moteur, rechargée à chaque modification. « Jouer depuis ici » (sur un nœud) démarre avec l'état qu'un joueur aurait à cet endroit. Pour jouer hors de l'éditeur : `cargo run -p orbit_player -- mon_jeu` (natif) ou `cargo xtask web` puis `cargo xtask serve` (navigateur).

## Exporter

Un export produit un jeu que les joueurs lancent **sans rien installer** : un `.zip` (Windows), un `.tar.gz` (Linux) ou un dossier à mettre sur un site web.

### Depuis l'éditeur

**Fichier > Exporter le jeu…** ouvre le panneau Export :

1. **Profil d'export** : nom du jeu (titre de fenêtre, noms de fichiers), **identifiant** (dossier des sauvegardes des joueurs : minuscules, chiffres, `-`, `_` ; ne le changez plus après publication), version, icône (PNG carré, 256x256 conseillé, relatif au projet), langue par défaut, compression du pak (LZ4 ou aucune), fichiers `.br` / `.gz` pour le web. « Enregistrer les profils » les écrit dans `orbit.project.ron` ; on peut en avoir plusieurs (`demo`, `release`...).
2. **Cibles** : Windows, Linux, Web. Une cible dont le **template** est introuvable est signalée.
3. **Dossier de sortie** (par défaut `export/` dans le projet).
4. **Exporter** : la barre de progression avance, le **journal** liste les étapes et les avertissements (assets inutilisés, traductions incomplètes...), **Annuler** arrête. L'histoire non enregistrée est enregistrée d'abord. Les erreurs de l'histoire (panneau Problèmes) bloquent l'export avec la liste.
5. À la fin : **Ouvrir le dossier**, et pour le web **Tester dans le navigateur** (un petit serveur local est lancé sur un port libre).

### En ligne de commande

```sh
orbit build  --project mon_jeu [--out dossier] [--profile release]   # valide et écrit <id>.pak
orbit export windows --project mon_jeu --out dist                    # dist/<id>-<version>-windows.zip
orbit export linux   --project mon_jeu --out dist                    # dist/<id>-<version>-linux.tar.gz
orbit export web     --project mon_jeu --out dist --serve            # dist/<id>-<version>-web/ puis http://127.0.0.1:<port>
orbit export all     --project mon_jeu --out dist                    # les cibles du profil
```

Options : `--profile <nom>`, `--templates <dossier>`, `--port <n>` et `--no-open` (avec `--serve`). Code de sortie 1 si l'histoire est invalide ou si un asset utilisé manque.

### Les templates de player

Un export ne compile rien : il colle votre jeu (un fichier `.pak`) à un **player précompilé**. Ces templates sont cherchés, dans l'ordre : le dossier choisi dans le panneau Export (ou `--templates`), la variable `ORBIT_TEMPLATES`, le dossier `templates/` à côté de l'éditeur ou de `orbit` (les installeurs le livrent), puis `target/templates` pour un arbre de build.

Les construire (développeurs d'OrbitEngine) :

```sh
cargo install wasm-bindgen-cli --version 0.2.129 --locked   # la version de Cargo.lock
cargo install wasm-opt --locked                              # facultatif : wasm 7 % plus petit
cargo xtask templates                                        # player de l'OS courant + player web dans target/templates
```

Le template **Linux** se construit sur une machine Linux (la CI le produit) : depuis Windows, `--target x86_64-unknown-linux-gnu` n'aboutit que si l'éditeur de liens de la cible est installé.

### Ce que reçoit le joueur

* **Windows** : `<Jeu>/<Jeu>.exe` seul (le jeu est dans l'exe), `README.txt`, licence de la police. L'icône du *fichier* .exe reste celle d'OrbitEngine (changer les ressources d'un exécutable compilé n'est pas possible sans outil externe) ; la fenêtre, elle, affiche l'icône du jeu.
* **Linux** : `<Jeu>/<id>` (exécutable) ; il faut un pilote GPU Vulkan ou OpenGL.
* **Web** : le dossier `<id>-<version>-web/`, à mettre tel quel sur un hébergeur statique (itch.io, GitHub Pages, Netlify...) ; voir `HOSTING.txt` à l'intérieur (HTTP(S), requêtes `Range`, `application/wasm`). Le jeu se charge **à la demande** : au départ la page n'a que le player (~1,4 Mo en brotli) et l'index ; les images et sons arrivent scène par scène, la scène suivante est préchargée. Plein écran dans les préférences ou F11 ; la musique se met en pause quand l'onglet perd le focus, et la partie est sauvegardée quand il est caché.
* Les sauvegardes sont rangées sous l'identifiant du jeu : `%APPDATA%\OrbitEngine\<id>` (Windows), `~/.local/share/OrbitEngine/<id>` (Linux), `localStorage` du site (web).
