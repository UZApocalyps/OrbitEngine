# Plugins de l'éditeur

Les plugins de l'éditeur ajoutent des panneaux, des fenêtres, des entrées de menu et des outils à l'éditeur lui-même. Ce sont des bibliothèques natives (un `.dll` sous Windows, `.so` sous Linux, `.dylib` sous macOS) que l'éditeur charge à son démarrage. L'éditeur est livré sans aucun plugin : vous installez ceux que vous voulez. **Studio IA** en est un.

Ils sont différents des *plugins Rust* des chapitres précédents, qui ajoutent des étapes à un jeu et sont exportés avec lui. Un plugin de l'éditeur ne se retrouve jamais dans un jeu exporté.

## Installer un plugin

1. Ouvrez **Outils > Extensions > Plugins…**.
2. Cliquez sur **Installer depuis un fichier…** et choisissez un paquet de plugin (`.orbitplugin`) ou une bibliothèque (`.dll`, `.so`, `.dylib`).
3. Lisez l'avertissement et confirmez : un plugin est du code natif qui s'exécute dans l'éditeur avec les mêmes droits que lui (vos fichiers, le réseau), sans bac à sable. N'installez que des plugins de sources de confiance.
4. L'éditeur vérifie le fichier (voir *Compatibilité* plus bas). S'il est accepté, il est copié dans le dossier des plugins et activé.
5. Cliquez sur **Redémarrer maintenant** dans le bandeau : les plugins sont chargés au démarrage de l'éditeur, jamais pendant qu'il tourne.

Chaque plugin de la liste affiche son nom, sa version, son auteur, sa description et son état :

- **Activé — en cours d'exécution** : chargé pendant cette session.
- **Activé — démarre après un redémarrage** : installé ou activé depuis le démarrage de l'éditeur.
- **Désactivé** : installé mais pas chargé.
- **Incompatible** : compilé pour une autre version de l'éditeur ; le message dit ce qui diffère. Recompilez le plugin, ou installez la version faite pour cet éditeur.
- **Erreur** / **Désactivé après un plantage** : la raison est affichée ; recochez **Activé** pour réessayer après une correction.

La case **Activé**, **Désinstaller** (qui supprime aussi les données du plugin) et l'installation d'une nouvelle version prennent effet au prochain démarrage. **Réglages** ouvre la page de réglages du plugin quand il en a une. **Ouvrir le dossier des plugins** montre où ils sont rangés :

- Windows : `%APPDATA%\OrbitEngine\Editor\plugins\`
- Linux : `~/.config/orbit-engine/editor/plugins/`

Un dossier par plugin (`plugins/<id>/`) contient sa bibliothèque, ses textes (`locales/`), ses fichiers (`assets/`) et ses propres données (`data/`, conservées lors des mises à jour). `plugins.ron` retient quels plugins sont activés.

## Quand un plugin se comporte mal

- Un plugin qui plante pendant qu'il tourne est arrêté et désactivé, avec une notification ; l'éditeur continue. Ses panneaux affichent alors l'erreur.
- Si l'éditeur lui-même s'arrête pendant le chargement d'un plugin, ce plugin est désactivé au démarrage suivant et le gestionnaire de plugins l'indique.
- Lancez l'éditeur avec `--no-plugins` (mode sans échec) pour ne charger aucun plugin ; le gestionnaire de plugins reste disponible pour en désactiver ou en désinstaller un.

## Compatibilité : recompiler à chaque version de l'éditeur

Un plugin et l'éditeur partagent directement des types Rust (l'interface egui, le projet, le contexte de l'éditeur). Ce n'est sûr que si les deux ont été compilés à partir des mêmes sources, par le même compilateur, avec les mêmes réglages. L'éditeur lit donc une **empreinte** dans chaque plugin avant de le charger, et refuse toute différence :

| Champ | Signification |
|---|---|
| éditeur | la version de l'API de l'éditeur (`orbit_editor_api`) |
| ABI des plugins | la version du contrat des plugins |
| compilateur | le `rustc` exact (version, date et commit) |
| egui | la version d'egui |
| plateforme | la cible (`x86_64-pc-windows-msvc`...) |
| profil de compilation | `release` ou `debug` |
| sources de l'API | une empreinte des sources des crates de l'API |
| disposition des types | une empreinte de la taille des types partagés |
| compilation des dépendances | une empreinte de l'identité (`TypeId`) des types partagés, qui dépend de la résolution des dépendances par cargo (`Cargo.lock`, features) |

Le manifeste d'un paquet peut aussi indiquer les versions de l'éditeur qu'il accepte (`min_editor`, `max_editor`). En pratique : **un plugin doit être recompilé pour chaque nouvelle version de l'éditeur**, avec la chaîne de compilation de l'éditeur.

Avant le chargement, l'éditeur vérifie aussi que le fichier est une bibliothèque pour ce système et ce processeur, lit l'empreinte dans un processus séparé de courte durée (une bibliothèque défectueuse ne peut pas faire planter l'éditeur), contrôle l'identifiant du plugin et compare la bibliothèque avec la somme de contrôle (SHA-256) enregistrée à l'installation : une bibliothèque modifiée ensuite est refusée.

## Studio IA

Studio IA est un plugin officiel compilé à partir de ce dépôt :

```
cargo build --release -p orbit_editor
cargo xtask plugins
```

La seconde commande écrit `target/plugins/orbit.ai_studio-<version>-<os>-<arch>.orbitplugin`. Installez ce fichier depuis le gestionnaire de plugins de l'éditeur compilé par la première commande (même profil : `cargo xtask plugins --debug` pour un éditeur en debug). Ses réglages (fournisseurs, clés) restent dans `ai_studio.ron` dans le dossier de configuration de l'éditeur, comme avant.

## Pour les programmeurs : écrire un plugin

Le modèle est `examples/editor_plugins/hello` : une fenêtre, une action du menu **Outils** qui ajoute une note au projet en une seule étape annulable, une page de réglages et ses textes en français et en anglais.

**Cargo.toml** : une `cdylib` qui ne dépend que du SDK, à la version de l'éditeur.

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
orbit_editor_sdk = { git = "https://.../OrbitEngine", tag = "v0.3.1" }

[lints.rust]
unsafe_code = "deny"   # pas "forbid" : export_plugin! génère des fonctions exportées
```

Hors de ce dépôt, copiez le `Cargo.lock` et le `rust-toolchain.toml` de l'éditeur à côté de votre `Cargo.toml` : le SDK dépend de l'éditeur pour que cargo résolve chaque crate partagée exactement comme pour l'éditeur (le champ *compilation des dépendances*).

**Code** :

```rust
use orbit_editor_sdk::native::{NativePlugin, PluginHost};
use orbit_editor_sdk::{ActionDef, WindowDef, egui};

#[derive(Default)]
pub struct Hello;

impl NativePlugin for Hello {
    fn on_load(&mut self, host: &mut PluginHost<'_>) -> Result<(), String> {
        host.action(ActionDef::new("hi", "hello.hi", |ctx| ctx.info("salut")).menu("tools", 900));
        host.window(WindowDef::new("window", |ctx, ectx| { /* egui::Window::new(...).show(ctx, ...) */ }));
        Ok(())
    }
}

orbit_editor_sdk::export_plugin!(Hello, id = "acme.hello", name = "Hello",
    description = "...", author = "ACME");
```

Ce que propose `PluginHost` : des panneaux (`panel`), des fenêtres flottantes (`window`), des actions avec entrées de menu, raccourcis et palette (`action`, `menu`), des inspecteurs (`inspector`), une page de réglages (`settings_page`), des tables de textes (`texts`, plus les `locales/<langue>/*.ron` du paquet, chargées par l'éditeur), le registre du projet pour les outils MCP et les hooks (`project`), un `EditorPlugin` existant (`install`), les dossiers du plugin (`plugin_dir`, `data_dir`, `editor_config_dir`).

Dans les closures, l'`EditorContext` lit le projet et le modifie comme les outils intégrés : `exec`, `transaction` et `group` (plusieurs opérations en une seule étape d'annulation), `call_tool` / `take_tool_result` (les outils MCP, une étape d'annulation par appel), `spawn_task` sur `state()` pour le travail en arrière-plan avec progression et annulation, `t` / `tf` pour les textes, `info` / `warn` / `error` pour les notifications.

**Paquet** : un `.orbitplugin` est un zip avec `manifest.ron` (`id`, `name`, `version`, `description`, `author`, `library`, et en option `min_editor` / `max_editor` / `homepage`), la bibliothèque, et des dossiers `locales/` et `assets/` facultatifs. `cargo xtask plugins --examples` compile et empaquette les plugins de ce dépôt, après avoir vérifié que chacun résout les crates partagées avec l'éditeur exactement comme lui.

**Règles** :

- Dessinez avec `orbit_editor_sdk::egui`, jamais avec votre propre egui.
- N'utilisez jamais un `egui::Context` depuis un fil d'arrière-plan : signalez la progression avec le `Progress` de la tâche et laissez l'interface dessiner.
- Tout ce que le plugin enregistre est protégé : une panique désactive le plugin au lieu de fermer l'éditeur. Les types de nœuds et les importeurs (de simples fonctions) ne le sont pas : ils ne doivent pas paniquer.
- Ne définissez pas d'allocateur global. La bibliothèque n'est jamais déchargée pendant que l'éditeur tourne.
