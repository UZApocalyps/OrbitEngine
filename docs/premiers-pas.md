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
