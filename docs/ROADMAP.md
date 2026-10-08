# OrbitEngine : plan de projet

Moteur de visual novel en Rust, équivalent à Ren'Py en plus libre : **éditeur graphique no-code avant tout**, animations 2D (tweens, timeline, squelettique), scènes 3D composées dans l'éditeur, export Windows / Linux / Web (Android plus tard). Scripting avancé en fin de roadmap.

Les cas limites sont marqués **[edge]**.

**Principe directeur :** tout ce qui est ajouté au moteur doit être faisable depuis l'éditeur. Une fonctionnalité sans interface d'édition n'est pas terminée.

---

## 1. Décisions d'architecture

### 1.1 Moteur maison sur wgpu + winit (abandon de Bevy)
- **Décision :** aucun framework moteur imposé. On écrit un moteur maison sur `wgpu` + `winit`, assemblé à partir de **bibliothèques briques** (non-frameworks), chacune remplaçable derrière une de nos crates :
  - math : `glam` ;
  - fenêtre et input : `winit` (+ `gilrs` pour les manettes) ;
  - rendu : `wgpu` (Vulkan, DX12, Metal, WebGL2 ; WebGPU en option) ;
  - texte : `cosmic-text` (+ éventuellement `glyphon`) ;
  - audio : `kira` (compatible wasm) ;
  - images : `image` ; glTF : `gltf` ;
  - éditeur : `egui` + `egui-wgpu` + `egui-winit` 0.36 (alignés sur wgpu 30 / winit 0.30 ; exigent Rust 1.95) + `egui_dock` + `egui-snarl` + `egui-phosphor` (ADR 0009) ;
  - plateforme web : `wasm-bindgen`, `web-sys`, `web-time`.
- **Raison :** ne dépendre d'aucun framework dont il faudrait attendre les fonctionnalités, ni subir ses changements d'API.
- **Scène : ADR à trancher au M0** (ECS ou graphe de scène).
  - **Recommandation : graphe de scène maison simple.** Un VN a peu d'entités (quelques dizaines) ; un ECS n'apporte rien et complique l'éditeur, l'undo et la sérialisation.
  - Alternative : `hecs` comme simple stockage, sans scheduler ni framework.
- **UI en jeu :** système de widgets et de thèmes maison, rendu par `orbit_render2d`, data-driven et éditable dans l'éditeur (cf. 1.2).
- **Coût de ce choix (honnête) :** il faut écrire soi-même :
  - le renderer 2D : batching de sprites, calques, render targets pour les transitions ;
  - le renderer 3D PBR avec ombres ;
  - le gestionnaire d'assets asynchrone ;
  - la boucle de jeu ;
  - l'abstraction plateforme (desktop, web, Android).
  - C'est plusieurs jalons de travail avant d'égaler ce qu'un framework fournirait. On l'accepte pour le contrôle total.
- **Mitigation :**
  - Renderer conçu **par couches** : `orbit_gfx` (bas niveau wgpu) puis `orbit_render2d` / `orbit_render3d`. Chaque couche est testable seule.
  - **wasm testé en CI dès le M0** : le rendu navigateur n'est jamais une surprise tardive.
  - La logique VN (modèle, VM, sauvegardes) reste dans des crates sans GPU, compilables en wasm.
- **[edge] Web = WebGL2.** Pas de compute shaders, lumières et ombres limitées, limites de textures et d'uniform buffers à respecter dès `orbit_gfx`. WebGPU reste un bonus (backlog).

### 1.2 UI de l'éditeur et UI en jeu
- **Éditeur : egui** via `egui-wgpu` et `egui-winit`, avec `egui_dock` pour les panneaux et `egui-snarl` pour le graphe de nœuds (versions et système de plugins : [ADR 0009](adr/0009-architecture-editeur.md)).
  - L'éditeur utilise la même `orbit_platform` et le même `orbit_gfx` que le player.
  - La preview est **le vrai runtime, rendu dans une texture wgpu** affichée dans un panneau egui. Le WYSIWYG est exact.
  - **Rendu à la demande** (pas de redessin continu hors preview en lecture) et **extensible par plugins** : les outils intégrés sont eux-mêmes des plugins (ADR 0009).
- **UI en jeu : widgets et thèmes maison** (textbox, boutons, menus, sliders, listes), rendus par `orbit_render2d`.
  - Layouts et styles data-driven (RON), entièrement éditables dans l'éditeur de thème.
  - On n'utilise pas egui en jeu : il est inadapté à l'habillage artistique d'un VN.
- **[edge] Texte riche** (machine à écrire, ruby, balises inline, CJK, RTL) : valider `cosmic-text` dès le M1. Prévoir un fallback de polices et des polices réduites pour le web.

### 1.3 Format de données
- **Modèle canonique = IR Rust sérialisée en RON**, un fichier par scène.
  - Chaque nœud a un **ID stable** (ULID) : saves, localisation et diffs en dépendent.
  - Les positions du graphe sont stockées dans un sidecar `.layout` (`story.layout` à côté de `story.ron`).
  - Chaque fichier porte un `schema_version`, avec les migrations associées, dès le M0.
- `orbit build` compile le RON en un pack binaire (`postcard`). Le RON est rechargé à chaud en dev.
- **DSL texte optionnel** (M8) : c'est une vue import/export de l'IR, jamais la source canonique.
- **Assets :** chaque asset a un ID stable (manifest ou `.meta`), jamais un chemin brut.
  - **[edge]** Une lint détecte les écarts de casse (Windows y est insensible, Linux et le web non).
- **Édition à deux niveaux :**
  - un graphe macro (scènes, branches) ;
  - un éditeur de scène linéaire avec timeline.
  - Un nœud par réplique serait ingérable.
- **Scènes 3D :** décrites en données (hiérarchie, transforms, matériaux, lumières, caméras) dans le même format RON, avec des références d'assets par ID.

### 1.4 Scripting (reporté au M8)
- **Cible :** Rhai + plugins Rust. Le scripting n'est pas nécessaire au MVP ni aux jalons 3D : tout passe par l'éditeur no-code.
- **Stratégie en 2 couches :**
  1. **Rhai** (Rust pur, compatible wasm) : conditions, variables, logique de jeu.
  2. **Plugins Rust natifs, compilés statiquement** (`orbit_plugin_api`), sans dylib. L'export avec plugins = `cargo build` du player.
- **Plugins de l'éditeur** (M3a, ADR 0009) : `orbit_plugin_api` (sans egui) et `orbit_editor_api`, compilés statiquement ; l'extension de l'IR passe par `NodeKind::Custom`. Le M8 ajoute le côté runtime / player complet.
- Option future desktop uniquement : hosting C# ou plugins WASM sandboxés (wasmtime, backlog, aucun engagement ; C# ne fonctionne pas sur le web).
- **Invariant dès le M0, pour ne pas bloquer l'ajout ultérieur du scripting :**
  - L'état de jeu n'est modifiable que **via l'API du VM** (store snapshotté).
  - Le **RNG est seedé** et sauvegardé.
  - **[edge]** Un effet externe futur devra être réversible ou marqué « non annulable » (rollback).

### 1.5 Runtime vs éditeur
- Le player exporté ne dépend jamais de l'éditeur, ni d'egui.
- **Règle de CI :**
  - Les crates core (`orbit_core`, `orbit_story`, `orbit_vm`, `orbit_save`, `orbit_i18n`) compilent en `wasm32-unknown-unknown` et se testent **sans GPU**.
  - Pas de `std::thread`, `std::fs` ni `Instant` direct hors de `orbit_platform` : tout passe par ses abstractions (temps via `web-time`, stockage, threads).
  - `orbit_gfx` et `orbit_platform` compilent aussi en wasm (WebGL2) et sont vérifiés en CI.

Crates du workspace :
- `orbit_core` : IR, IDs, variables, expressions, schéma versionné et migrations.
- `orbit_story` : chargement RON, validation, compilation en pack.
- `orbit_project` : API de commandes unique sur un projet (ouverture, commandes typées sur l'IR, validation, undo/redo, événements de changement, versions par nœud) ; sans GPU, stockage derrière un trait (cf. 1.6). Utilisée par l'éditeur, `orbit_cli` et `orbit_mcp`.
- `orbit_vm` : exécution déterministe pas à pas, snapshots, rollback, texte lu, RNG seedé.
- `orbit_save` : saves et trait de stockage (fichiers, IndexedDB, Android).
- `orbit_i18n` : IDs de chaînes, traductions, PO/CSV.
- `orbit_assets` : manifest, IDs stables, pak, gestionnaire de chargement async.
- `orbit_platform` : fenêtre, boucle de jeu, input (clavier, souris, tactile, manette), temps, stockage, abstraction desktop / web / Android.
- `orbit_gfx` : couche wgpu bas niveau (device, surfaces, pipelines, buffers, textures, render targets, profils de qualité).
- `orbit_render2d` : sprites, batching, calques, transitions, mesh 2D, UI en jeu.
- `orbit_render3d` : renderer PBR, lumières, ombres, skybox, post-process simple, caméras.
- `orbit_scene` : graphe de scène maison et modèle de données des scènes 2D et 3D (hiérarchie, matériaux, lumières, caméras) ; indépendant du GPU.
- `orbit_text` : mise en forme (`cosmic-text`), machine à écrire, ruby, CJK, RTL, atlas de glyphes.
- `orbit_audio` : musique, effets, voix, mixage, fades (`kira`).
- `orbit_ui` : widgets et thèmes en jeu (data-driven, rendus par `orbit_render2d`).
- `orbit_anim` : tweens, timeline, spritesheets, animation squelettique 2D (os, skinning, poids).
- `orbit_runtime` : runtime maison qui orchestre plateforme, rendu, audio, texte et VM (pont VM ↔ scène, transitions).
- `orbit_script` : Rhai, API du VM, sandbox (M8).
- `orbit_dsl` : parser et printer du DSL texte (M8).
- `orbit_plugin_api` : plugins Rust, partie sans GPU ni egui (manifeste, nœuds `Custom`, validateurs, importeurs, outils MCP, hooks ; handlers runtime complétés au M8). Créée au M3a, `API_VERSION` 0 tant que seuls des plugins internes existent.
- `orbit_editor_api` : partie egui des plugins (panneaux, actions, inspecteurs, vues de nœuds) ; M3a.
- `orbit_player` : binaire du jeu (desktop, web, Android).
- `orbit_editor` : application egui (`egui-wgpu` / `egui-winit`), hôte des plugins ; les outils intégrés sont des plugins.
- `orbit_mcp` : serveur MCP (SDK `rmcp`) exposant l'API de `orbit_project` à une IA ; headless ou intégré à l'éditeur ; jamais dans le player (cf. 1.6).
- `orbit_cli` : `validate`, `build`, `export`, `run`, `test-story`, `mcp`.
- `orbit_export` : templates par plateforme, bundle web.
- `xtask` : automatisation.

### 1.6 Pilotage par IA : serveur MCP
- **Invariant posé dès le M0 :** toute modification d'un projet passe par l'**API de commandes unique** d'`orbit_project` (ouverture de projet, commandes typées sur l'IR, validation, undo/redo, événements de changement).
  - L'éditeur ET le serveur MCP utilisent cette même API : les éditions de l'IA sont validées, annulables et visibles dans l'éditeur.
  - **Aucun accès direct aux fichiers RON par le MCP** (ni par l'éditeur : il passe par la même API).
  - Prolonge le principe directeur : une fonctionnalité éditable à la main l'est aussi par l'IA, via les mêmes commandes.
- **Crate `orbit_mcp`** basée sur le SDK Rust officiel `rmcp` (licence permissive à vérifier via `cargo deny` avant adoption ; sinon repli sur un protocole JSON-RPC minimal maison).
- **Deux modes :**
  - **headless** (M3f) : `orbit mcp --project <dir>`, transport stdio, sans GPU, travaille sur les fichiers du projet via `orbit_project`.
  - **live** (M4) : l'éditeur ouvert expose le MCP en local (HTTP streamable sur 127.0.0.1). Les modifs de l'IA apparaissent en direct, comme un collaborateur, **regroupées en une entrée d'undo par appel d'outil**.
- **Surface exposée :**
  - *Resources* : manifest du projet, liste des scènes / personnages / assets, schéma de l'IR, résultats de lint.
  - *Tools* : lire une scène ; créer / modifier / supprimer nœuds et lignes de dialogue, choix, branches ; personnages et expressions ; importer un asset ; valider ; build / export ; lancer une preview depuis un nœud ; avancer le VM ; lire l'état du VM et les variables ; **capture d'écran de la preview** pour que l'IA « voie » le rendu (mode live).
  - Outils étendus plus tard : animation 2D, rig, scène 3D (M5, M6), scripting (M8) ; **chaque plugin fournit ses propres outils** (`McpToolDef` d'`orbit_plugin_api`, utilisables en headless).
  - Live : l'UI et le MCP postent leurs commandes dans la même `CommandQueue` (ADR 0009 §7).
- **Schémas d'outils générés depuis les types de l'IR**, même source que l'inspecteur auto-généré (§2 M3) : pas de divergence entre IR, éditeur et MCP.
- **[edge] Sécurité :**
  - écoute uniquement sur localhost, **désactivé par défaut**, activation explicite dans l'éditeur ;
  - jeton d'accès obligatoire ;
  - périmètre limité au dossier du projet (aucun accès fichier arbitraire ; chemins normalisés, liens symboliques hors projet refusés) ;
  - opérations destructrices (suppression de scène, export écrasant) signalées comme telles dans les métadonnées de l'outil ;
  - journal des actions de l'IA consultable dans l'éditeur.
- **[edge] Conflits :** en mode live, les commandes de l'IA et de l'utilisateur sont **sérialisées dans la même file**. Un appel portant sur un nœud modifié entre-temps échoue proprement (version / ETag par nœud) au lieu d'écraser.
- **Le MCP n'est jamais embarqué dans le player exporté** (règle de CI : `orbit_player` ne dépend ni d'`orbit_mcp` ni de `rmcp`).

---

## 2. Jalons

### M0 : Fondations
- Workspace, licence provisoire, ADR (`docs/adr/`) reprenant les décisions du §1 (dont graphe de scène vs `hecs`).
- `rust-toolchain.toml`, `cargo deny`, versions de `wgpu` et `winit` épinglées.
- CI Windows, Linux et wasm32 : build, clippy, tests.
- `orbit_platform` : fenêtre, boucle, input, temps, stockage minimal, desktop et navigateur.
- `orbit_gfx` : device, surface, pipeline de sprite, texture. Contraintes WebGL2 respectées dès le départ.
- IR minimal (Say, Choice, Jump, Label, SetVar, If) avec ULID, `schema_version` et loader RON.
- Invariant d'état : modifié uniquement via l'API du VM, RNG seedé.
- `orbit_project` (squelette) : ouverture de projet et API de commandes unique sur l'IR (commandes typées, validation, événements de changement) ; invariant « aucune modification hors de cette API » (cf. 1.6). Undo/redo complet au M3.
- Test d'architecture : core et vm compilent en wasm et se testent sans GPU.
- **Fin :** la CI est verte sur les 3 cibles et un sprite texturé s'affiche en natif ET dans le navigateur (WebGL2).

### M1 : Renderer 2D, texte, audio, VM minimal : un dialogue joué depuis RON
- **Statut : fait.** `orbit_player` joue `examples/stories/demo.ron` (27 répliques, 2 personnages, 1 choix, fond, musique, transitions fade/dissolve) en natif et sur le web (`orbit_runtime` + `orbit_ui`), avec écran « cliquer pour démarrer », écran de fin et écran d'erreur lisible. Hot reload natif par polling de la date du fichier (`orbit_assets::FileWatcher`), redémarrage au début de l'histoire.
- **Dettes connues :**
  - pas de manette (`gilrs` absent de `orbit_platform`) ni d'événements tactiles ; les répétitions de touches ne sont pas filtrées (Espace maintenu = avance en rafale) ;
  - le hot reload ne recharge que l'histoire : un asset modifié nécessite un redémarrage (textures en cache par chemin) ; pas de reprise au label courant ;
  - le `Theme` est sérialisable (RON) mais le player n'en charge pas encore depuis un fichier ; textes moteur en français non localisés ;
  - le format d'histoire n'expose que fade et dissolve (le masque de `orbit_render2d` n'est pas encore pilotable depuis le RON) ;
  - polices CJK/arabe absentes du bundle web (lourdes) : texte non latin à valider sur le web au M2 ;
  - audio non vérifié à l'oreille sur le web (l'état « audio started » est vérifié) ; la démo utilise du WAV, l'OGG est décodé mais non exercé par la démo ;
  - skip, auto, historique, saves et rollback : livrés au M2.
- `orbit_render2d` : batching de sprites, calques, render targets (base des transitions), caméra 2D.
- `orbit_text` : textbox, machine à écrire, avance du texte. Validation CJK/RTL.
- `orbit_audio` : musique avec boucle et fade, effets sonores.
- `orbit_assets` : chargement asynchrone.
- `step()` pur renvoyant `WaitInput`, `WaitTime`, `WaitTransition` ou `End` ; call/return ; variables et conditions.
- Stage : fond, sprites avec expressions, positions nommées. Choix au clic, au clavier et à la manette.
- Transitions fade et dissolve (shader de masque sur render targets).
- IDs de chaînes stables dès maintenant, pour la future localisation.
- `orbit validate` et hot reload.
- **[edge]** Aucune I/O synchrone dans le VM. Écran « cliquer pour démarrer » à cause de l'autoplay audio bloqué sur le web.
- **Fin :** ~20 répliques lues depuis un RON, 2 personnages, 1 choix, fond, musique et transition, en natif et sur le web.

### M2 : Fonctionnalités VN de base
- **Statut : fait.** Le player (natif et web) enchaîne chargement, « cliquer pour démarrer », menu principal, jeu et écrans (pause, sauvegarde, chargement, préférences, historique), avec rollback, skip, auto, saves multi-slots à miniatures, autosave, préférences persistées, i18n à chaud et variables persistantes. Architecture : `Shell` (machine d'état sans GPU dans `orbit_runtime`), voir ADR 0008. Tests automatisés sans GPU de la machine d'état, du rollback (stage + audio), du skip/auto, des flux save/load, de l'autosave et des persistants ; vérifié à la main en natif et sur le web (menu, partie, save avec miniature, rollback, historique, load, préférences en français, rechargement de page, Continuer).
- Snapshots et état de scène déclaratif. Rollback avec restauration de l'audio : la musique de la scène est relancée si elle diffère, laissée telle quelle sinon.
- Save/load multi-slots avec miniatures et autosave ; stockage fichiers (natif) et `localStorage` (web ; IndexedDB repoussé, cf. ADR 0008).
- **[edge]** Saves entre versions : hash de l'histoire et IDs stables. Un slot incompatible est grisé, un slot endommagé donne une notification, sans crash.
- Skip (tout ou déjà lu), auto, historique.
- Variables persistantes vs variables de partie.
- Menu principal, pause, préférences, thèmes de niveau 1 (système de widgets maison).
- i18n runtime.
- Tests sans GPU : rollback, round-trip save/load, tests de propriété.
- **Fin :** la suite de tests automatisés passe en CI.
- **Dettes courtes :**
  - `localStorage` : environ 5 Mo par origine, une save avec miniature pèse 30 à 60 Ko ; à la saturation l'écriture échoue proprement (notification) ; passage à IndexedDB si cela devient un problème ;
  - la langue système n'est détectée que si l'OS la fournit (`sys-locale`) ; le fuseau horaire natif vient de `time` (UTC s'il est indisponible, notamment sous Unix multithread) ;
  - polices CJK/arabe toujours absentes du bundle web : le texte non latin d'une traduction n'est pas affiché correctement ;
  - pas de manette ni de tactile ; répétitions de touches non filtrées ;
  - ~~`UiTexts` du `Theme`~~ : retiré (ADR 0011) ;
  - les textes de l'interface n'existent qu'en anglais et en français (autres langues : anglais), surchargeables par jeu via `RuntimeConfig::ui_text_overrides` mais sans fichier de thème pour l'instant ;
  - sons d'UI : branchés (`Theme::sounds`, chemins relatifs à la racine des assets) mais aucun son par défaut ni test à l'oreille ;
  - la miniature est le décor et les personnages (sans textbox) rendus à 320x180 au moment de la demande de save ; la dernière autosave faite à la fermeture de la fenêtre n'a pas de miniature (pas de relecture GPU possible à ce moment) ;
  - la liste des traductions est un fichier `i18n/index.txt` à tenir à jour (le web ne peut pas lister un dossier) ;
  - sur le web, l'autosave de fermeture n'existe pas (un onglet qui se ferme ne prévient pas l'application) : seules les autosaves de progression et les préférences (écrites au fil de l'eau) sont garanties ;
  - le quick menu chevauche le bas de la textbox ;
  - l'écran de fin renvoie toujours au menu principal (plus de « recommencer » direct).

### M3 : Éditeur v1, sans code (M3a → M3f)
Architecture : [ADR 0009](adr/0009-architecture-editeur.md) (statut Accepté). Principes : egui 0.36 sur `orbit_platform` (un seul `Gfx`), **outils intégrés = plugins** (`orbit_plugin_api` sans egui, `orbit_editor_api` avec egui), rendu à la demande, toute modification par `Command`. Éditeur natif uniquement (Windows, Linux).

**M3a : socle. Statut : fait.**
- Relever `rust-toolchain.toml` et `rust-version` à 1.95 (MSRV d'egui 0.36) ; CI 3 cibles verte ; dépendances egui sans `default_fonts`, `cargo deny` vert.
- `orbit_platform` : mode `OnDemand` (`ControlFlow::Wait` / `WaitUntil`, `HostRequest::RequestRedraw`) et hook d'événements fenêtre bruts, sans changer le player.
- Crates `orbit_plugin_api`, `orbit_editor_api`, `orbit_editor` (binaire) ; registres (panneaux, actions, menus, raccourcis, inspecteurs, types de nœuds, importeurs, outils MCP, hooks) ; `EditorContext` (lecture du projet, commandes seulement, sélection). Chaque outil intégré est un plugin. `xtask check-arch` mis à jour.
- App egui : thème maison clair / sombre, Inter, icônes `egui-phosphor`, dock `egui_dock` (disposition par défaut et persistance), palette de commandes (Ctrl+K / Ctrl+Shift+P), menus et raccourcis issus du registre.
- Ouvrir / créer un projet (dossier, projets récents), sauvegarde atomique (`ProjectStorage`), autosave et recovery, file watching.
- Undo/redo global branché (menu Édition avec libellés, `Command::label()`), `CommandQueue` locale utilisée par l'UI, état d'éditeur (sélection, layout) séparé du projet.
- Schéma IR v4 : variante `NodeKind::Custom { plugin_id, kind, version, data }` + migration v3 → v4, aller-retour sans perte d'un nœud dont le plugin est absent.
- **Fin :** (1) test sans GPU : un plugin d'essai enregistre une action, un panneau et un type de nœud, la palette le trouve et un `Custom` survit à un aller-retour RON ; (2) l'éditeur ouvre `examples/stories/demo.ron`, on modifie une réplique, annule / rétablit, sauvegarde, et `orbit validate` passe ; (3) kill brutal puis relance propose la restauration ; (4) mesures : première frame < 1 s sur le projet démo, **0 frame dessinée en 5 s d'inactivité** (compteur testé).
- **Réalisé (M3a)** : les quatre critères de fin sont couverts par des tests sans GPU (`crates/orbit_editor/tests/editor.rs`, 14 tests : plugins et palette, aller-retour d'un `Custom`, édition / annulation / sauvegarde / validation, récupération après arrêt brutal, confirmation à la fermeture, disposition persistante, raccourcis clavier, **absence de redessin au repos** à plusieurs tailles de fenêtre) et vérifiés avec le binaire : première frame **≈ 570 à 670 ms** (build release, GPU AMD / Vulkan, démo ouverte), 3 frames au démarrage puis 0 en idle (compteur `--frame-counter`), captures de la frame rendue dans `target/screenshots/`. Crates : `orbit_plugin_api`, `orbit_editor_api`, `orbit_editor` (binaire `orbit-editor`). `orbit_platform` : mode à la demande et hook d'événements bruts, player inchangé. IR v4 (`NodeKind::Custom`, migration v3 → v4 triviale). Toolchain 1.95. Détails et écarts : ADR 0009, « Notes d'implémentation du M3a ».
- **Dettes M3a** (à reprendre ; rien ne bloque M3b) :
  - file watching des fichiers du projet (rechargement / conflit) : fait en M3e ;
  - `CommandQueue` locale pour l'UI (prévue pour le MCP live, M4) : l'UI appelle pour l'instant `Project` directement via `EditorContext` ; seule une file de requêtes d'éditeur (`EditorRequest`) existe. **M4 :** le MCP live a sa propre file thread-safe (`orbit_mcp::live::CommandQueue`) vidée sur le fil de l'éditeur ; l'UI ne la partage pas encore (dette M4) ;
  - disposition et sélection propres au projet (`.layout`, `.orbit/workspace.ron`) : seule la disposition globale est persistée ;
  - autosave « à l'inactivité » (seul le délai de 30 s existe) ;
  - navigation (scènes, nœuds, personnages, assets) dans la palette et recherche globale (Ctrl+Shift+F) : faites en M3e (la navigation au clavier de la palette l'était déjà) ;
  - raccourcis reconfigurables par l'utilisateur (le registre est prêt, pas le fichier) ; échelle d'interface réglable ;
  - pas de police à chasse fixe (Inter) ni de police CJK / arabe dans l'éditeur (le texte des répliques japonaises s'affiche en carrés) ;
  - le modèle de projet « copie de la démo » n'est pas proposé : seul le projet minimal l'est (la démo s'ouvre depuis `examples/stories/demo.ron`) ;
  - sauvegarder une histoire ouverte réécrit le RON sans ses commentaires ;
  - la liste de l'explorateur est reconstruite à chaque frame (virtualisée à l'affichage, mais pas incrémentale) : à surveiller sur 5000 nœuds (critère M3c) ;
  - l'archive portable est produite par la CI (job `editor`, zip Windows et tar.gz Linux), mais n'a pas encore été exécutée sur un vrai runner ; pas d'installeur (M4) ;
  - le build Linux est vérifié par `cargo check` / `clippy` croisés depuis Windows, pas exécuté.

**M3b : preview intégrée du runtime. Statut : fait.**
- `Runtime::render_to_target` et `Runtime::load_story(Arc<Story>)` ; preview dans un panneau egui par texture enregistrée sur le device partagé ; entrées routées seulement au focus ; saves de preview isolées.
- Hot reload depuis l'état mémoire du projet (debounce ~150 ms), reprise au nœud courant si son ID existe ; invalidation du cache de texture d'un asset modifié.
- « Jouer depuis le début » et « jouer depuis ici » : recherche de chemin bornée sur le VM headless (`snapshot` / `restore`, RNG seedé), repli sur un panneau de presets d'état (variables, décor).
- Barre de lecture (lecture / pause / redémarrer, vitesse), erreurs de chargement lisibles dans le panneau ; redessin continu uniquement si la preview joue et est visible.
- **Fin :** modifier une réplique met la preview à jour en < 200 ms sans la redémarrer ; test sans GPU : l'état du VM obtenu par « jouer depuis ici » sur un nœud de `demo.ron` égale celui d'une lecture complète jusqu'au même nœud ; preview en pause = 0 frame ; capture asynchrone de la cible de preview fonctionnelle (réutilisée par le MCP).
- **Réalisé (M3b)** : tous les critères de fin sont couverts. `orbit_vm::seek` (recherche de chemin bornée, en largeur : le moins de choix d'abord, déterministe) ; `orbit_runtime::preview` (départ « depuis le début » / « depuis ce nœud » / reprise après modification, sans GPU) ; `Runtime::new_embedded`, `load_story(Arc<Story>, racine d'assets)`, `render_to_target`, `inject_input`, `tick`, `is_animating` / `take_dirty` (demande de frame), commandes de lecture ; `Renderer2d` : rendu et transitions dans une cible, vue « gamma » d'une cible pour egui ; plugin intégré Preview (`orbit_editor::builtin::preview`) et hôte GPU (`orbit_editor::preview_host`). Tests sans GPU : chemin « jouer depuis ici » égal à une lecture complète (nœud de branche, conditions et variables, nœud de départ, inatteignable, borne atteinte, déterminisme) ; reprise après modification (même nœud, texte modifié, nœud supprimé → label le plus proche, aucun repère → début, histoire terminée) ; 0 frame quand le jeu attend (shell) ; anti-rebond, taille de la cible, mapping des touches, racine d'assets, assets remplacés ; panneau (focus, entrées, actions, plein panneau). Mesures (build release, GPU AMD / Vulkan, démo) : **modification de texte → preview à jour ≈ 100 ms** (100 ms d'anti-rebond + chargement < 1 ms, mesuré par le log `orbit_editor::perf`), « jouer depuis ici » < 1 ms sur la démo, **0 frame en 7 s** de jeu en attente d'un clic (preview à l'arrêt de la frappe, sans mouvement de souris) ; première frame de l'éditeur ≈ 0,6 s avec la preview. Captures : `target/screenshots/m3b-*.png`. Détails et écarts : ADR 0009, « Notes d'implémentation du M3b ».
- **Dettes M3b** (à reprendre) :
  - panneau de **presets d'état** (variables et décor imposés à la main) pour forcer un cas quand « jouer depuis ici » ne trouve pas de chemin : non fait, le repli est l'état par défaut avec un avertissement dans la console ;
  - la recherche de chemin distingue les points de choix par variables et pile d'appels, pas par décor ni sprites : un chemin qui ne diffère que par le décor n'est pas exploré en plus (borne : 4 000 points de choix, 200 000 pas) ;
  - un **asset modifié** est détecté à chaque rechargement et à chaque lecture par la date de modification du fichier ; depuis M3e le watcher du projet recharge aussi la preview dès qu'un fichier change ;
  - pas de traductions dans la preview (langue source), ni de choix de la langue ;
  - la disposition par défaut n'est appliquée qu'aux nouvelles installations : un `config.ron` existant ne contient pas le panneau Preview (Affichage > Afficher : Preview, ou « Réinitialiser la disposition ») ;
  - la preview n'est pas un second moteur : un `Custom` sans gestionnaire est ignoré comme dans le player (avertissement de validation à l'export, M4) ;
  - la musique n'est pas mise en pause mais coupée (volume maître à 0) pendant la pause ; elle reprend là où elle en est au retour ;
  - la capture de la cible existe (`PreviewHost::start_capture`, option `--preview-capture`) ; **M4 :** exposée au MCP par l'outil `preview_screenshot` (cible dédiée de 1280×720, PNG en contenu image) ;
  - pas de test GPU en CI (aucun GPU sur les runners) : `Runtime::render_to_target`, la texture partagée et le rechargement sont vérifiés à la main (captures) ; la logique de décision est testée sans GPU.

**M3c : éditeur de scène linéaire, personnages, inspecteur auto-généré. Statut : fait.**
- Catalogue de commandes étendu (insérer / déplacer / dupliquer / remplacer / supprimer, setters par type de nœud, personnages et expressions, variables, scènes, `SetCustomData`) avec inverses et tests d'undo.
- Schéma de l'IR via `schemars` (feature `schema` d'`orbit_core`, compatible wasm) ; inspecteur auto-généré depuis ce schéma avec indications d'UI (sélecteurs d'assets, de personnages, de nœuds).
- Éditeur de scène : liste virtualisée des nœuds (dialogues, choix, décor, sprites, musique, transitions, variables, attente), ajout par menu et par palette, glisser-déposer, édition en place, raccourcis d'écriture rapide (Entrée = nouvelle réplique).
- Éditeur de personnages : nom, couleur, sprites, expressions, aperçu.
- Éditeur de variables (portée jeu / persistante) et de conditions (sans écrire d'expression à la main pour les cas simples).
- **Fin :** un testeur construit sans toucher à un fichier une scène de 20 répliques avec 2 personnages, un choix, un décor et une musique, qui se lance dans la preview ; chaque `NodeKind` est éditable (test d'aller-retour par type) ; scène de 5000 nœuds défilée sans chute sous 60 fps.

- **Réalisé (M3c)** : `orbit_project::ops` (insertion, suppression avec nettoyage des textes, déplacement par bloc, Alt+↑/↓, duplication, copier / coller avec nouveaux identifiants et références internes recalculées, scènes, personnages, expressions, variables, ressources ; une étape d'annulation par geste) ; commandes ajoutées `MoveScene` et `SetPositions` (celles de `ext.rs` du M3f sont réutilisées pour le reste ; `SetNodeKind` refuse de transformer une étiquette encore ciblée ; l'inverse de `SetVariable` suit le renommage) ; `Character::default_position` (optionnel, absent du fichier si non défini). `orbit_editor_api::fields` (boîte à outils de champs typés réutilisable par les plugins) ; inspecteurs enregistrés pour chacun des 16 types intégrés par le même registre que les plugins, éditeur générique des données d'un `Custom` ; panneaux Scène, Personnages, Variables ; validation en direct par nœud (`Diagnostics`). Tests sans GPU (`orbit_project/tests/ops.rs`, `orbit_editor/tests/scene_editor.rs`) : chaque opération + undo / redo, copier / coller, refus de suppression, inspecteur dessiné pour chaque `NodeKind`, renommages, **scène de 20 répliques / 2 personnages / un choix / un décor / une musique construite par les seules opérations de l'éditeur, validée, enregistrée, rechargée et jouée dans le VM**. Mesure (build release, GPU AMD / Vulkan, 5007 nœuds, défilement continu 40 px / frame) : **0,74 ms de CPU par frame en moyenne (p95 1,3 ms)**, ~1270 frames / s présentées. Captures : `target/screenshots/m3c-*.png`. Détails : ADR 0009, « Notes d'implémentation du M3c ».
- **Dettes M3c** :
  - pas de schéma `schemars` : l'inspecteur est fait de champs typés écrits à la main (`fields`), pas généré depuis l'IR ; à unifier avec les schémas MCP ;
  - hauteur des lignes de réplique fixe (2 lignes de texte, le reste est tronqué avec « … ») ; pas de retour à la ligne automatique de hauteur variable ;
  - glisser-déposer testé au niveau des opérations et de la géométrie, pas par événements souris simulés ; pas de glisser vers une autre scène (menu / couper-coller) ;
  - ~~l'ajout d'une ressource se fait par un sélecteur de fichier ; import complet, miniatures asynchrones, audio~~ : fait en M3e ;
  - ~~miniatures synchrones~~ : asynchrones depuis M3e ;
  - ~~validation complète relancée à chaque révision~~ : hors du fil d'interface et anti-rebondie depuis M3e ;
  - Ctrl+Z dans un champ de texte de l'inspecteur utilise encore l'annulation du champ (dans l'édition en place d'une réplique, c'est l'historique du projet) ;
  - la disposition par défaut n'est appliquée qu'aux nouvelles installations (panneaux Scène, Personnages, Variables : menu Affichage).

**M3d : graphe de scènes. Statut : fait.**
- Panneau graphe sur `egui-snarl` derrière un trait `GraphView` (remplaçable) : **blocs** comme nœuds (la tête d'une scène et chaque étiquette), arêtes calculées depuis `Jump` / `JumpScene` / `Choice` (une sortie par option) / `If` (vrai / faux) / `Call` et l'enchaînement d'un bloc au suivant **dans la même scène** ; positions dans le sidecar `.layout`.
- Création et édition par le graphe via commandes (une étape d'annulation par geste) : nouvelle scène, étiquette, choix avec ses branches, condition, saut de scène, relier / rediriger / couper un lien, renommer, supprimer (avec confirmation) ; double-clic ouvre la scène ; mini-carte, zoom, recadrage, recherche, détection des blocs orphelins.
- **Décision prise pendant le M3d : les scènes ne s'enchaînent plus.** Une scène ne se poursuit jamais dans la suivante : on passe de l'une à l'autre uniquement par un nœud explicite `JumpScene { scene, label }` (« Saut de scène »). Une scène dont l'exécution atteint la fin sans saut, `End` ni `Return` **termine la partie** (avertissement de validation « la partie se termine ici », « fin de partie » dans le graphe). Schéma IR **v5** ; la migration v4 → v5 ajoute un `JumpScene` vers la scène suivante non vide à la fin de chaque scène qui « tombait » dans la suivante, sauf si sa dernière ligne est un `Jump`, un `Choice`, un `End` ou un `Return`.
- **Fin :** depuis le graphe seul, on crée un choix avec deux branches qui mènent à deux scènes différentes, jouable dans la preview ; les positions persistent ; 200 nœuds fluides en pan / zoom (sinon repli documenté sur un graphe maison).
- **Réalisé (M3d)** :
  - `orbit_project::graph` : modèle « macro » du graphe calculé depuis l'IR en `O(nœuds)` (blocs, emplacements de sortie : suite, saut, saut de scène, option, vrai / faux, appel ; arêtes ; cibles manquantes ; blocs orphelins par parcours depuis le début de l'histoire ; nœuds jamais exécutés après un saut). `orbit_project::graph_ops` : opérations d'édition (créer une scène ou une étiquette, relier, rediriger, couper un lien, ajouter un choix ou une condition, terminer un bloc, renommer, supprimer avec plan de ce qui est perdu), chacune étant un `Project::group` (une entrée d'undo, tout ou rien). `orbit_project::graph_layout` : sidecar `<histoire>.layout` (`story.layout` à côté de `story.ron`, RON, jamais dans l'histoire), disposition automatique en couches (parcours en profondeur depuis le début, plus long chemin, barycentre, repli en plusieurs rangées pour garder un format d'écran, orphelins en grille dessous) et placement des seuls blocs sans position.
  - `orbit_editor::builtin::graph` : panneau **Graphe** (onglet à côté de Scène) ; `GraphView` (trait) et `SnarlView` (egui-snarl 0.12.0, *vendored* : voir dettes). Blocs colorés par type (scène, étiquette, choix, condition, fin, retour, appel), icônes Phosphor, résumé (nombre de répliques, personnages, première réplique), pastille de validation par bloc, une broche de sortie par option / branche / saut. Gestes : tirer une broche vers un bloc (relier ou rediriger), la lâcher dans le vide (nouvelle scène liée), clic droit sur un lien ou une option (couper, éditer le texte, ajouter une option), menu du bloc (ouvrir, jouer depuis ici, ajouter un choix ou une condition, terminer, renommer, supprimer), glisser pour déplacer ; sélection synchronisée avec l'explorateur, l'inspecteur et la preview ; F recadre, mini-carte, recherche. Actions de palette `orbit.graph.*` (recadrer, réorganiser, nouvelle scène, ajouter un choix ou une condition au bloc sélectionné, centrer, chercher). Les positions ne sont écrites qu'après un déplacement ou une action sur le graphe (regarder le graphe n'écrit aucun fichier).
  - IR v5 : `NodeKind::JumpScene`, validation (scène ou étiquette cible inexistante, étiquette hors de la scène cible : erreurs ; scène qui atteint sa fin : avertissement), VM (fin de scène = fin de partie ; `JumpScene` exécuté, pris en compte par le rollback, `seek` et « jouer depuis ici »), `orbit_project` (supprimer une scène ciblée par un saut de scène est refusé comme pour les étiquettes ; le copier / coller recalcule l'étiquette visée), `orbit_mcp` (`add_command type=jump_scene`), `orbit_save` (tag de hash 18), éditeur de scène (ligne « Saut de scène », bouton et double-clic pour aller à la scène cible), inspecteur (sélecteur de scène et d'étiquette filtrée par scène), exemples migrés (`demo.ron` passe `orbit validate --check-files` sans erreur ni avertissement).
  - Tests sans GPU : modèle de graphe (sauts, choix, conditions, appels, sauts de scène, suite limitée à la scène, orphelins, cibles manquantes), chaque édition + undo / redo, persistance et rechargement du layout (et sidecar illisible), disposition automatique sans chevauchement (jusqu'à 200 blocs, format d'écran), **critère de fin : un choix à deux branches créé depuis le graphe seul, qui mène à deux scènes différentes, validé et jouable dans le VM**, confirmation de suppression, 200 blocs dessinés ; migration v4 → v5, validation, VM (scènes qui ne s'enchaînent plus, saut de scène vers le début ou une étiquette, rollback, seek), MCP.
  - Mesure (build release, GPU AMD / Vulkan, 200 blocs, pan et zoom continus avec `--autoscroll 3`) : **4,1 à 4,5 ms de CPU par frame en moyenne (p95 5,5 à 6,8 ms, ~220 à 240 frames / s présentées) ; à 1000 blocs : 33 ms par frame (~30 frames / s), limite de l'absence de culling**. Captures : `target/screenshots/m3d-*.png` (démo, 200 blocs, zoom, édition d'un choix). Générateur : `orbit-editor --generate-graph <n> <fichier.ron>`. Détails : ADR 0009, « Notes d'implémentation du M3d ».
- **Dettes M3d** (à reprendre) :
  - `egui-snarl` 0.12.0 (et sa dépendance `egui-scale` 0.6.1) sont **vendus** dans `vendor/` (sources inchangées, seul le `Cargo.toml` diffère : `egui` en `default-features = false`) parce que la version publiée active `default_fonts` d'egui, dont les polices (OFL, UFL) sortent de `deny.toml` ; à retirer dès qu'une version amont rend ces features optionnelles. Repli prévu si Snarl devient un frein : le trait `GraphView` isole le panneau ;
  - Snarl dessine tous les blocs à chaque frame (pas de culling) : 200 blocs sont fluides, un graphe de plusieurs milliers demandera un culling ou le graphe maison ;
  - les gestes de souris (tirer un lien, le lâcher dans le vide, déplacer) sont testés au niveau des événements du widget (`apply_event`) et vérifiés sur captures statiques, pas par événements souris simulés ;
  - la condition créée depuis le graphe vaut `true` : son expression s'édite dans l'inspecteur / l'éditeur de scène, pas dans le graphe ; de même pour la condition d'une option ;
  - les liens `Jump` / `Call` / option / branche peuvent encore viser une étiquette d'une autre scène (c'est valide) ; seule la suite d'un bloc relié à une autre scène devient un saut de scène ;
  - créer une étiquette à la fin d'une scène fait « tomber » le bloc précédent dans le nouveau s'il n'est pas terminé (l'IR est linéaire) ;
  - les scènes créées depuis le graphe ne contiennent qu'une étiquette : elles terminent la partie (avertissement) tant qu'on ne les complète pas ;
  - zoom et défilement ne sont pas mémorisés dans le `.layout` (positions seulement) ; les positions ne sont pas annulables (état d'éditeur) ;
  - la disposition automatique ne minimise pas les croisements au-delà de quatre passes de barycentre ; les très longs liens entre rangées traversent le graphe ;
  - la migration v4 → v5 change la structure (nœuds ajoutés) des histoires qui s'enchaînaient : les saves existantes de telles histoires peuvent être refusées par la vérification de structure d'`orbit_save`.

**M3e : assets, validation en direct, recherche. Statut : fait.**
- Explorateur de projet et d'assets, import (copie dans le projet, `AssetId` stable, manifest, lint de casse), importeurs fournis par plugin, miniatures paresseuses hors thread UI avec cache borné, prévisualisation audio.
- Console de validation en direct (`orbit_core::validate` déboncée et incrémentale), navigation par clic vers le nœud fautif, avertissements (assets manquants, scènes orphelines, plugin absent).
- Recherche globale (Ctrl+Shift+F) sur répliques, personnages, variables, assets ; palette étendue à la navigation.
- Éditeur de thème de l'UI en jeu : inspecteur auto-généré sur le `Theme` RON avec preview (peut glisser vers le backlog sans bloquer la fin de M3).
- **Fin :** importer un PNG et un audio par glisser-déposer les rend utilisables dans l'éditeur de scène ; une erreur introduite est affichée en < 300 ms et son clic sélectionne le nœud ; la recherche trouve une réplique sur un projet de 10 000 nœuds en < 100 ms.
- **Réalisé (M3e)** : `orbit_editor::assets` (import de fichiers et de dossiers : décodage par les importeurs enregistrés via `orbit_plugin_api`, copie sous `assets/<type>/` avec nom sûr et unique sans collision de casse, `AssetId` stable, **une étape d'annulation par import**, refus propres ; renommage de fichier avec journal de déplacements que l'annulation et le rétablissement rejouent ; lint de casse et de fichiers manquants ; scan du dossier `assets/`), importeurs intégrés (PNG / JPEG, WAV / OGG, par décodage ; un son long devient une musique), `orbit_project::ops::{remove_asset, set_asset_path, is_valid_asset_path}` et `asset_users`, **dette M3c corrigée : `SetAsset` (None) refuse aussi un asset utilisé par une expression de personnage** (`CommandError::AssetInUseByCharacter`), `orbit_project::sandbox` (ex-`orbit_mcp::sandbox`, partagé : toute écriture de l'import passe par le bac à sable de chemins), `orbit_editor::thumbs` + `builtin::thumbs` (miniatures hors du fil d'interface : 2 fils, file la plus récente d'abord, cache disque `.orbit/thumbs/` borné à 3000 fichiers, cache de textures borné à 400, chargement paresseux des seules lignes visibles), panneau **Assets** (grille / liste virtualisées, filtres, recherche, tri, écoute, renommer, retirer avec confirmation, « non déclaré » + « Déclarer », glisser vers les champs ressource de l'inspecteur et les lignes Décor / Musique / Son), `orbit_editor::watch` (surveillance par fil d'arrière-plan, changement reporté une fois stable ; `orbit_platform::Waker` réveille la boucle sans polling, donc toujours 0 frame au repos), rechargement proposé d'une histoire modifiée hors de l'éditeur (conflit si modifications non enregistrées), `orbit_editor::diagnostics::Live` (validation inline ou en arrière-plan avec anti-rebond), `Diagnostics::problems` (avec `Location`), panneau **Problèmes**, pastilles de la barre d'état, `EditorContext::reveal`, `orbit_editor::search` (index plat dépliant répliques, choix, personnages, variables, étiquettes, scènes, assets ; reconstruit par révision), panneau **Recherche** (Ctrl+Maj+F), navigation dans la palette (scènes, personnages, assets, répliques ; les correspondances éparpillées de commandes passent après). Tests sans GPU : import (copie, nom, ID stable, manifeste, undo / redo, un fichier déjà dans le projet, dossier déposé, collisions de casse), refus (faux PNG, PNG tronqué, WAV invalide, format inconnu, fichier absent : rien n'est copié, une erreur par fichier), chemins hors projet (`..`, absolu, lecteur, `\`, extension changée), renommage (références intactes, annuler / rétablir déplacent le fichier), suppression refusée (nœud **et** expression de personnage, au niveau de la commande), lint (manquant, casse, collision de casse), watcher (ajout / modification / suppression, changement stabilisé, écriture de l'éditeur ignorée), rechargement externe (avec et sans conflit, « garder les miennes » ne redemande pas), validation en direct sur 10 000 nœuds (latence mesurée, une frame ne bloque jamais), révélation d'un problème, recherche (pertinence, accents, mots multiples, 10 000 nœuds). **Mesures** (build release, GPU AMD / Vulkan, 10 007 nœuds) : **erreur affichée 5,5 ms après la modification** (première modification après une pause ; au pire 83 ms juste après une autre validation, anti-rebond de 80 ms ; validation seule 3,3 ms sur le fil d'arrière-plan), **recherche : 1,2 ms la requête la plus lente** (index de 10 000 nœuds construit en 3,6 ms) ; en debug 116 ms et 5,5 ms. Éditeur au repos avec le panneau Assets ouvert : 0 frame. Captures : `target/screenshots/m3e-*.png`. Détails : ADR 0009, « Notes d'implémentation du M3e ».
- **Dettes M3e** (à reprendre) :
  - **éditeur de thème de l'UI en jeu** : repoussé au backlog (cf. `docs/TODO_LATER.md`, « Moderniser le thème de base du jeu »), la ROADMAP l'autorise sans bloquer la fin de M3 ;
  - pas de type d'asset **police** dans l'IR (`AssetKind` = image, musique, son) : le filtre « police » de la bibliothèque et l'import de polices attendent la prise en charge des polices de jeu par le runtime ;
  - le type d'un audio importé est déduit de sa durée (≥ 20 s : musique, sinon son) ; le champ ressource d'un nœud musique / son le reclasse à l'ajout par fichier, mais la bibliothèque n'a pas encore d'action « changer le type » ; l'import par le MCP (`import_asset`) ne décode toujours pas les fichiers (les importeurs intégrés sont ceux de l'éditeur, pas ceux de `orbit mcp`) ;
  - annuler un import retire la déclaration au manifeste mais **laisse le fichier copié** (il réapparaît « non déclaré » ; le rétablir le redéclare avec le même identifiant) ; « retirer » ne supprime jamais le fichier du disque ;
  - miniatures : pas de forme d'onde pour l'audio (icône et durée) ; les images sont décodées en entier à la première demande (pas de décodage à échelle réduite) ;
  - le watcher compare date et taille toutes les 500 ms (pas de `notify` : un remplacement d'un fichier par un autre de même taille et de même date n'est pas vu ; le dossier surveillé est `assets/`, pas l'arbre entier du projet) ; la recharge d'une histoire modifiée dehors repart d'un historique d'annulation vide et recharge la preview depuis le début ;
  - l'index de recherche est reconstruit en entier à chaque révision où il est utilisé (3,6 ms pour 10 000 nœuds en release) plutôt que mis à jour nœud par nœud ; il ne cherche pas dans les noms de locuteurs des répliques ni dans les valeurs d'expressions ; pas de remplacement global ;
  - la validation n'est pas incrémentale : elle est complète, mais hors du fil d'interface et anti-rebondie (3 ms sur 10 000 nœuds) ; les validateurs de plugins et la liste des plugins absents tournent sur le fil d'interface (le registre n'est pas `Send`) ;
  - les nouveaux panneaux (Assets, Problèmes, Recherche) ne sont ajoutés qu'aux nouvelles installations (menu Affichage ou « Réinitialiser la disposition » sinon) ; pas de navigation au clavier dans le panneau Problèmes (la souris et F8 « problème suivant »).

**M3f : MCP headless. Statut : fait.**
- `orbit_mcp` (`rmcp` 3.5, Apache-2.0) et `orbit mcp --project <dir>` sur stdio, sans GPU, via `ProjectHost` ; resources (manifest, scènes, personnages, assets, schéma de l'IR, lint) ; outils de lecture et d'édition de l'histoire, import d'asset et validation, tous via les commandes d'`orbit_project` ; outils fournis par les plugins (`McpToolDef`) ; schémas générés depuis l'IR (même source que l'inspecteur) ; journal des actions.
- Une commande refusée renvoie une erreur structurée, sans modification partielle ; destructifs signalés. La `CommandQueue` de M3a est prête pour le mode live (M4).
- **Fin :** un client MCP réel (Claude Code) crée une courte scène avec choix qui passe `orbit validate` ; test de CI comparant schémas MCP et types de l'IR ; `orbit_player` ne dépend ni d'`orbit_mcp` ni de `rmcp` (vérifié par `check-arch`).
- **Réalisé (M3f)** : crate `orbit_mcp` sur `rmcp` 3.5.0 (Apache-2.0, `cargo deny` vert sans nouvelle exception) ; `orbit mcp --project <dossier|fichier.ron>` (stdio, sans GPU) ; **sauvegarde atomique après chaque appel réussi, une entrée d'undo par appel** (`Project::group`, annulation complète si l'appel échoue, y compris pour les outils de plugins) ; 22 outils (`list_scenes`, `read_scene`, `create_scene`, `rename_scene`, `delete_scene`, `add_dialogue`, `edit_dialogue`, `add_choice`, `add_command` pour décor / show / hide / musique / son / transitions / attente / set_var / if / jump / jump_scene / label / call / return / end / custom, `delete_node`, `move_node`, `list_characters`, `create_character`, `edit_character`, `list_variables`, `declare_variable`, `list_assets`, `import_asset`, `validate`, `simulate`, `undo`, `redo`, `save`) et les outils des plugins (`plugin_id.outil`) ; 7 ressources + une par scène, dont le **schéma JSON de l'IR généré depuis les types Rust** (`schemars` sous la feature `schema` d'`orbit_core`) ; schémas d'entrée des outils générés des types d'arguments ; sécurité (bac à sable de chemins avec liens symboliques résolus, `destructiveHint` sur `delete_scene` / `delete_node`, journal `.orbit/mcp.log`, erreurs structurées, paniques interceptées, conflits de version par nœud) ; nouvelles commandes d'`orbit_project` (scènes, déplacement, remplacement de nœud, personnages, variables, assets) dans `ext.rs`. Documentation : [docs/mcp.md](mcp.md). Tests : unitaires par outil, sécurité, robustesse, cohérence schéma MCP / IR (échoue si un `NodeKind` n'a pas d'outil) et **bout en bout** (le binaire `orbit mcp` en sous-processus, `initialize` → `tools/list` → scène à deux personnages et choix à deux branches → `validate` → `simulate` des deux branches → le fichier passe `orbit validate`).
- **Dettes M3f** (à reprendre) :
  - la validation « client réel » a été faite avec un client JSON-RPC scripté (même protocole que Claude Code), pas avec une session Claude Code interactive ;
  - pas de surveillance des fichiers : un projet modifié sur disque pendant que le serveur tourne sera écrasé à la prochaine sauvegarde ; **M4 :** le verrou `.orbit/lock` refuse `orbit mcp` sur un projet que l'éditeur tient (`--force`), et le mode live donne un seul propriétaire du projet, avec jeton, journal dans l'éditeur, capture d'écran de la preview ;
  - les plugins intégrés de l'éditeur ne sont pas chargés par `orbit mcp` (seuls ceux installés via `HeadlessHost::install_plugin`) ;
  - `ProjectHost` est un trait de transport (liste / appel d'outils, ressources) : **M4 :** l'implémentation live (`LiveHost`, file de commandes de l'éditeur) est écrite ;
  - pas d'outils pour supprimer un personnage / une variable / un asset (les commandes existent), ni pour éditer les nœuds autres que les répliques (supprimer puis recréer ; `Command::SetNodeKind` est prête) ;
  - les contenus d'assets ne sont pas inspectés à l'import MCP (seule l'existence du fichier est validée) ; l'import de l'éditeur, lui, décode (M3e) ;

**M3g : éditeur de vue (taille, miroir, position libre des personnages). Statut : fait (ajouté pendant le M4).**

- Besoin : régler la **taille** d'un personnage, l'**inverser** sur X ou Y et le placer **librement** (en plus de gauche / centre / droite et des positions nommées), dans un éditeur visuel qui montre la scène telle qu'elle est au nœud sélectionné. Placé dans la famille M3 (éditeur v1 : outils de scène) parce qu'il prolonge la preview du M3b (même runtime, même recherche de chemin) et l'inspecteur du M3c ; il ne dépend d'aucun travail du M4.
- **IR v6** ([ADR 0004](adr/0004-ir-ron-canonique-ids-ulid.md)) : `Show` gagne `scale`, `scale_y`, `flip_x`, `flip_y` ; `Position::Free { x, y }` ; `Character::default_scale`. Tout est omis du RON quand c'est la valeur par défaut ; migration 5 -> 6 triviale ; `examples/stories/demo.ron` migrée et utilisant miroir, échelle et position libre. Un `Show` sur un personnage déjà présent met à jour sa position, son échelle et son miroir (comportement du VM déjà en place, désormais testé) : pas de nœud « Déplacer » séparé. Validation : échelle finie dans 0,05 à 10, coordonnées libres finies dans -2 à 3.
- **VM / runtime / rendu** : `CharacterOnStage` et `PresentationEvent::Show` portent l'échelle effective, le miroir (et le `Show` d'origine) ; restaurés par rollback, chargement et « jouer depuis ici ». Un seul calcul placement -> sprite (`orbit_runtime::stage::character_placement`) partagé par le rendu et les gizmos ; échelle autour de l'ancre bas-centre ; **miroir par inversion des UV** (`Sprite2d::flipped`) : l'emprise du sprite ne change pas, le batching n'est pas affecté, pas de culling dans `orbit_render2d` (le test le verrouille) ; natif et web. Format des sauvegardes **v2** (décodeur v1 conservé, `orbit_save::legacy_v1`).
- **`orbit_project::ops`** : `ShowLook`, `show_look`, `show_look_command` (commande validée à appliquer avec `execute_merged` pour un glisser en direct), `set_show_look` (une entrée d'undo), `set_default_scale` ; `rename_expression` conserve désormais position, échelle et miroir.
- **MCP** : `add_command` (show), `add_dialogue`, `create_character` / `edit_character` acceptent `scale`, `scale_y`, `flip_x`, `flip_y`, la position libre `{x, y}` et `default_scale` ; le test de cohérence schéma / IR passe (`position` est un nom **ou** `{x, y}`).
- **Éditeur** : inspecteur du `Show` (échelle avec étirement vertical optionnel et retour au défaut, miroir X / Y avec icônes, sélecteur de position avec « Libre » et champs X / Y), échelle par défaut dans le panneau Personnages, et le panneau **Éditeur de vue** (plugin intégré `orbit.sceneview`, onglet à côté de Scène et Graphe) : le **vrai runtime** dessine le décor et les personnages au nœud sélectionné dans une render target (état obtenu par la même recherche de chemin que « jouer depuis ici », puis le nœud est exécuté une fois), avec des gizmos egui dessinés des mêmes nombres. Le personnage du `Show` sélectionné est cadré avec ses poignées ; **glisser** le déplace (le `Show` devient une position libre, mise à jour en direct, preview rechargée à chaud, **une entrée d'undo par glisser**, Échap annule) ; sept poignées d'échelle autour des pieds (Maj : garder le ratio) ; boutons Miroir X / Y ; **magnétisme** sur les positions nommées (et réécriture de la position nommée quand le glisser s'y pose exactement) et sur une grille de 5 %, Alt le désactive ; repères gauche / centre / droite, positions personnalisées et marges de sécurité 16:9 ; **flèches** (pas fin 0,2 %, Maj 2 %) ; un clic sur un autre personnage sélectionne le `Show` qui l'a placé. 0 frame au repos (la scène n'est redessinée que si l'histoire, le nœud, la taille ou un asset changent).
- **Tests** (sans GPU) : validation et migration (`orbit_core`), VM et sauvegardes (échelle / miroir au rollback, restore, save / load, v1 lisible), placement et miroir (`orbit_runtime::stage`, `orbit_render2d`), état de la scène à un nœud (`orbit_runtime::scene_view`), opérations et undo (`orbit_project`), outils MCP (`orbit_mcp`), géométrie des gizmos (`sceneview::geom` : poignées, ratio, bornes, magnétisme, flèches) et gestes complets à travers egui (`orbit_editor/tests/scene_view.rs` : glisser = position libre + une entrée d'undo, poignée d'échelle, étirement, miroir, magnétisme, Alt, Échap, clic sur un autre personnage, flèches). Captures : `target/screenshots/sceneview-*.png`.
- **Dettes M3g** (à reprendre) :
  - le miroir et l'échelle sont des états du `Show` : pas d'animation (un déplacement progressif d'un personnage viendra avec les tweens du M5) ;
  - l'étirement non uniforme est exposé (inspecteur, poignées d'arête, MCP) mais pas de rotation ;
  - l'éditeur de vue ne règle que le `Show` sélectionné ; il ne déplace pas plusieurs personnages à la fois, et ne montre pas l'emplacement précédent d'un personnage qu'on déplace (« fantôme ») ;
  - le décor n'est pas modifiable dans la vue (sélectionner le nœud Décor montre le décor, sans gizmo) ;
  - la vue montre la scène au chemin le plus court qui mène au nœud (comme « jouer depuis ici ») ; sur un nœud inatteignable elle part de l'état par défaut, avec un avertissement ;
  - pas de capture de l'éditeur de vue exposée au MCP (celle de la preview l'est au M4) ;
  - les flèches font une entrée d'undo par pression (pas de fusion des répétitions) ;
  - le panneau n'est dans la disposition par défaut que pour les nouvelles installations (Affichage > Afficher : Éditeur de vue sinon).

**M3h : langue de l'éditeur (anglais / français). Statut : fait.**

- Tous les textes de l'éditeur passent par des **clés** résolues à l'affichage (tables RON embarquées, 1053 clés par langue, test de parité en / fr). Langue du système au premier lancement, choix dans Affichage > Langue, changement à chaud, option de debug `--lang`. Un plugin enregistre ses propres tables (`EditorRegistry::texts`). Détails : notes i18n de l'[ADR 0009](adr/0009-architecture-editeur.md).
- **Dettes M3h** : les libellés d'annulation et les lignes de journal créés par l'éditeur restent dans la langue du moment ; les messages techniques de `orbit_core` (types d'expressions) et d'`orbit_project` (erreurs d'E/S) restent en anglais ; les noms de manifeste des plugins ne sont pas traduits ; les histoires de bench ne le sont pas non plus.

**Fin de M3 :** un non-développeur crée une VN de 5 min uniquement depuis l'interface ; **et** une IA crée via MCP une courte scène avec choix qui passe `orbit validate`. Une archive portable de l'éditeur (zip / tar.gz) est produite par la CI.

### M4 : Export multiplateforme (jalon MVP)
- **Export des jeux (fait, voir plus bas)** : `orbit build` et pak d'assets, lazy loading HTTP sur le web ; export Windows (zip) et Linux (tar.gz) ; export web (wasm-bindgen, wasm-opt, brotli, loader avec barre de progression, serveur de test local) ; export lancé depuis l'éditeur (panneau, profils, journal) et par MCP.
- **Distribution de l'éditeur** (fait) : installeurs Windows (MSI WiX + NSIS) et Linux (AppImage + deb) via `cargo-packager` 0.11 (MIT / Apache-2.0, ADR 0009 §8), configurés dans `crates/orbit_editor/Cargo.toml` (`[package.metadata.packager]`) ; job CI `release` (tag `v*` ou `workflow_dispatch`) qui attache les installeurs à une GitHub Release **en brouillon** ; dossier `templates/` livré à côté de l'exécutable pour les templates de player ; version du workspace affichée dans la barre d'état ; installeur NSIS testé en réel (installation par utilisateur, lancement, désinstallation). Auto-update : backlog ([TODO_LATER](TODO_LATER.md)).
  - **Dettes** : binaires Linux (AppImage / deb) produits et vérifiés par la seule CI (config validée sous Windows) ; MSI inspecté (contenu, raccourcis, désinstallation) mais pas installé (installation par machine, droits administrateur) ; pas de signature de code (SmartScreen) ; le projet démo ouvert depuis le dossier d'installation y écrit `.orbit/` (cache de miniatures), à remplacer par une copie du démo dans le dossier utilisateur ; le désinstalleur laisse ces fichiers générés ; les templates de player sont remplis par la CI (job `templates`) ; pas de dépôt apt / winget.
- **MCP mode live** : l'éditeur expose le MCP en HTTP streamable sur 127.0.0.1 (désactivé par défaut, jeton, panneau de journal des actions de l'IA).
  - Modifs de l'IA visibles en direct, une entrée d'undo par appel d'outil, file de commandes commune avec l'utilisateur, versions par nœud.
  - Outils supplémentaires : preview depuis un nœud, avancer le VM, lire état et variables, **capture d'écran de la preview**, build / export (destructifs signalés).
  - **[edge]** Éditeur sans GPU disponible (CI) : les outils preview / capture renvoient une erreur explicite, le reste fonctionne.
- Budget de taille wasm mesuré en CI.
- **[edge]** IndexedDB, plein écran, DPI, focus et pause audio.
- Tests end-to-end en CI. Documentation « premiers pas ».
- **Fin :** le jeu du M3 tourne sur une machine propre pour chaque cible ; une IA connectée à l'éditeur ouvert modifie l'histoire en direct, voit la capture de la preview et lance un export via MCP.

**M4, MCP mode live. Statut : fait.**
- L'éditeur expose le MCP en **HTTP streamable sur 127.0.0.1** (`rmcp` 3.5 `transport-streamable-http-server` + `hyper` 1 / `hyper-util` pour l'écoute ; MIT / Apache-2.0, `cargo deny` vert **sans nouvelle exception**). **Désactivé par défaut** : activé depuis le panneau « Assistant IA » (port configurable, 7878 par défaut) ; `--mcp-port` / `--mcp-token` pour les scripts. **Jeton** de 256 bits (`getrandom`), masqué / copiable / régénérable, boutons « Commande Claude Code » (`claude mcp add --transport http … --header "Authorization: Bearer …"`) et « .mcp.json ». Requêtes sans jeton valide refusées (401), `Host` contrôlé contre l'anti-rebond DNS (403), écoute uniquement sur la boucle locale.
- **Exécution** : serveur sur un thread dédié (runtime tokio à un thread) ; chaque appel est posté dans une **file de commandes** (`CommandQueue`) vidée sur le **fil de l'éditeur** avant le dessin de l'UI, exécutée par `Project::group` (une entrée d'annulation « MCP : <outil> » par appel, tout ou rien), réponse par canal ; le `Waker` réveille la boucle. **`ProjectHost` a une implémentation « éditeur »** (`LiveHost`) et les outils sont **les mêmes qu'en headless** (`ToolRunner` extrait de `HeadlessHost`, zéro duplication). Modifications visibles en direct, **non enregistrées** (comme celles de l'utilisateur ; outil `save`), conflit avec une modification de l'utilisateur = `version_conflict` explicite.
- **Outils live** : `preview_play` (depuis un nœud), `preview_step` (N pas ou jusqu'au prochain choix), `preview_choose`, `preview_state` (nœud, réplique, variables, scène déclarative), `preview_stop`, **`preview_screenshot`** (PNG 1280×720 en contenu image MCP via la relecture asynchrone existante), `select_node`, `build_project` / `export_game` (**destructifs**, derrière `ProjectHost::export` : « non disponible » tant que `orbit_export` n'est pas dans l'éditeur). **[edge]** sans GPU : les outils preview renvoient `unavailable` avec un message explicite, le reste fonctionne (testé sur le cœur d'éditeur). `orbit_runtime` gagne `Runtime::advance_one` / `choose_option` / `snapshot` (mode embarqué).
- **Panneau « Assistant IA »** : état (arrêté / à l'écoute / client connecté), port, jeton, journal en direct (outil, arguments, résultat, durée, « révéler » le nœud touché, `.orbit/mcp.log`), indicateur discret (panneau et barre d'état), « Annuler la dernière action de l'IA » (jamais les modifications de l'utilisateur).
- **Cohérence avec le headless** : verrou `.orbit/lock` (battement de cœur, std seulement, dans `orbit_project::lock`), avertissements dans les deux sens ; `orbit mcp` refuse un projet tenu par l'éditeur (`--force` pour passer outre).
- **Tests** : intégration HTTP sans GPU (serveur réel sur un port libre, client HTTP écrit à la main, `EditorCore` : `initialize` → `tools/list` → édition ; modification du projet, entrée d'annulation, journal, refus sans jeton / mauvais jeton / jeton tronqué / mauvais schéma / `Host` étranger, conflit de version avec une modification de l'utilisateur, outils preview / export sans GPU, régénération du jeton, port occupé, verrou dans les deux sens) ; unitaires (jeton, comparaison, file, outils live, résolution des options, JSON d'état).
- **Validation réelle** : `orbit-editor examples/stories/demo.ron --mcp-port … --mcp-token …` avec un client HTTP scripté : ajout d'une réplique, preview depuis ce nœud, capture d'écran reçue par MCP (`target/screenshots/m4-mcp-capture.png`) et éditeur avec le panneau et son journal (`target/screenshots/m4-mcp-panel.png`).
- **Dettes M4 (MCP live)** (à reprendre) :
  - **export** : `build_project` / `export_game` sont branchés sur `orbit_export` (fait avec l'export des jeux) ; ils tournent sur le fil de l'éditeur (la fenêtre est occupée pendant un export lancé par l'IA) ;
  - l'**UI ne poste pas encore ses commandes dans la file** du MCP : même fil et même sérialisation, mais l'UI appelle `Project` directement (ADR 0009 §7 vise une file unique de `Command` typées) ;
  - `validate` (outil) sur une **histoire ouverte seule** vérifie les fichiers d'assets depuis le dossier de l'histoire, pas depuis le dossier d'assets retrouvé par l'éditeur (faux « fichier absent » sur la démo) ;
  - l'exécutable de l'éditeur exige un GPU : le cas « sans GPU » n'est exercé que sur le cœur d'éditeur (tests) ;
  - jeton en clair dans la configuration utilisateur, pas de jeton en lecture seule ni d'expiration ; pas de notifications serveur → client ; sessions HTTP non reprises après un redémarrage du serveur ;
  - validation « client réel » faite avec un client HTTP scripté (même protocole que Claude Code), pas avec une session Claude Code interactive ;
  - le panneau « Assistant IA » est ajouté au dock des nouvelles installations seulement (menu Affichage sinon).

**M4, export des jeux. Statut : fait.** Décisions et mesures : [ADR 0010](adr/0010-export-pak-et-templates.md).
- **Crates** : `orbit_pak` (pak versionné + pack de jeu postcard, sans I/O, wasm) et `orbit_export` (sans egui : `export`, `export_many`, `build`, `run_request` pour le MCP ; templates ; serveur de test local), utilisés par le CLI (`orbit build`, `orbit export <windows|linux|web|all> --project --out [--profile] [--serve]`), le panneau **Export** de l'éditeur (fil de travail + `Waker`, progression, journal, annulation, « Ouvrir le dossier », « Tester dans le navigateur ») et les outils MCP `build_project` / `export_game`.
- **Profils d'export** dans `orbit.project.ron` (`export_profiles` : nom du jeu, identifiant, version, icône, cibles, compression, langue par défaut ; optionnels).
- **Compilation** : validation bloquante, assets utilisés seulement (avertissement pour les autres), traductions embarquées. **Runtime** : lecture du pak natif (fichier, accolé à l'exe, `--pak`) et web (HTTP `Range`, index d'abord, assets par scène avec préchargement des scènes suivantes, jeu suspendu si la scène affichée attend) ; le mode développement (RON + fichiers) est inchangé.
- **Templates** : `cargo xtask templates` (player de l'OS, sans `showcase`, + player web `release-wasm` + wasm-bindgen + wasm-opt) ; l'éditeur les trouve dans `templates/` à côté de son exécutable (ou dossier configurable, `ORBIT_TEMPLATES`) ; la CI les construit et les met dans les installeurs.
- **Taille du wasm** : 10 167 000 → **4 426 429** octets (−56,5 %), brotli 1,36 Mo ; **budget** 4,8 Mo / 1,65 Mo brotli, vérifié en CI (`cargo xtask wasm-budget`).
- **[edge] web** : plein écran (API navigateur, F11), DPI (winit), pause audio sans focus, plus de rendu et **autosave** quand l'onglet est caché ou quitté (dette M2 réglée), `localStorage` conservé (décision et seuil de bascule vers IndexedDB dans l'ADR).
- **Tests** : unitaires (pak, pack, compilation, templates, serveur `Range`, profils, job de l'éditeur), `crates/orbit_export/tests/e2e.rs` (démo exportée pour chaque cible avec des templates factices : structure du zip, du tar.gz et du bundle web, pak identique partout ; export d'un projet invalide ; lancement du **vrai** player exporté natif quelques secondes depuis un dossier hors du dépôt, `--ignored`), test MCP `export_tools_run_the_real_export`. Vérifié à la main : bundle web exporté chargé dans un navigateur (loader, requêtes `Range` sur `game.pak`, partie jouable, sauvegarde en `localStorage`).
- **Dettes (export)** :
  - l'icône du **fichier** `.exe` Windows reste celle d'OrbitEngine (celle de la fenêtre est celle du jeu) ;
  - le template **Linux** n'est produit que par la CI (pas de cross-compilation depuis Windows) : non exécuté dans cet environnement (structure du tar.gz vérifiée avec un template factice) ; pas de macOS ;
  - le lancement du player natif exporté n'est pas automatisé en CI (il faut un GPU / un affichage) : test `--ignored` à lancer à la main ;
  - web : pas de zip du bundle (itch.io), pas de test automatisé du navigateur en CI (vérifié à la main), un hôte sans `Range` télécharge tout le pak ; les assets d'une scène non encore atteinte ne sont jamais évincés (mémoire) ; plusieurs scènes lourdes à la suite peuvent suspendre brièvement le jeu sur réseau lent ;
  - l'export lancé par le MCP bloque le fil de l'éditeur ; pas d'outil MCP de build / export en headless (`orbit export` en ligne de commande) ;
  - pas d'export incrémental (le pak est reconstruit en entier), pas de signature de code, pas de vérification que les traductions couvrent tous les écrans de l'UI ;
  - `wasm-opt` utilisé : binaryen 116 (crate `wasm-opt` 0.116) ; une version plus récente réduirait encore le wasm.

### M5 : Animations 2D (M5a tweens et timeline, M5b squelettique)
**M5a : tweens et timeline**
- Tweens (position, échelle, rotation, alpha, couleur) et éditeur de courbes d'easing.
- Commandes move, shake, zoom, flash, avec pistes parallèles.
- Timeline dans l'éditeur : pistes, clés, scrubbing.
- Spritesheets, personnages en calques, particules, parallax.
- **[edge]** Au rollback ou au skip, les animations sautent à leur état final.
- **Fin M5a :** une scène animée créée uniquement dans la timeline, correcte au rollback et sur le web.

**M5b : animation squelettique 2D maison**
- Modèle : os (hiérarchie, transforms), skinning de mesh 2D, poids par sommet, `orbit_anim`.
- Rendu dans `orbit_render2d` (mesh 2D déformé, skinning CPU ou GPU selon le budget WebGL2).
- **Éditeur de rig** : création des os, maillage depuis une image, peinture des poids, aperçu de déformation.
- **Éditeur d'animation** : clips squelettiques dont les clés vivent dans la même timeline que M5a.
- **Outils MCP :** création et édition de clips, pistes, clés, os et poids via les commandes d'`orbit_project`, schémas générés depuis l'IR.
- **[edge]** Rollback, skip et sauvegardes : l'état d'un clip est déclaratif (clip + temps), donc restaurable.
- Pas d'import Spine/Live2D : le format est maison (pas de problème de licence tierce).
- **Fin M5b :** un personnage riggé et animé entièrement depuis l'éditeur, joué en natif et sur le web.

### M6 : Renderer 3D et éditeur de scène 3D complet
- `orbit_render3d` : renderer PBR (metallic-roughness), lumières (directionnelle, point, spot), ombres, environnement / skybox, post-process simple (tonemapping, bloom, vignette éventuelle).
- Import glTF (maillages, matériaux, textures, hiérarchie) ; **animations glTF ignorées** pour l'instant.
- **Éditeur de scène 3D complet :**
  - viewport avec gizmos (déplacer, tourner, mettre à l'échelle) et grille ;
  - hiérarchie et inspecteur (transforms, visibilité) ;
  - éditeur de matériaux PBR avec aperçu ;
  - lumières, ombres, environnement / skybox ;
  - **caméras fixes** (position, FOV), sélectionnables depuis les commandes de scène.
- Intégration 2D/3D : sprites, personnages 2D et textbox rendus **par-dessus** une scène 3D ; transitions entre scènes 2D et 3D.
- **Outils MCP :** import glTF, hiérarchie, matériaux, lumières, skybox, caméras fixes ; capture de la preview 3D au profil choisi.
- **Pas d'animation 3D dans ce jalon** (ni clips glTF, ni travelling) : backlog « plus tard ».
- **Profils de qualité :**
  - **Web (WebGL2)** : nombre de lumières et d'ombres limité, une seule lumière à ombres, textures compressées (KTX2) et réduites, budget de polygones, pas de compute.
  - **Desktop** : plus de lumières, ombres de meilleure résolution, post-process complet, textures pleine résolution.
  - Le profil est choisi à l'export et avertit dans la console de validation si une scène le dépasse.
- **[edge]** Scène identique sur les deux profils : dégradation prévisible (ombres douces → dures, bloom coupé), jamais de crash.
- **Fin :** une scène 3D composée entièrement dans l'éditeur (modèles, matériaux, lumières, ombres, skybox, caméras fixes), avec dialogue 2D par-dessus, correcte au rollback, fluide dans le navigateur au profil web.

### M7 : Polish, localisation outillée, accessibilité, Android
- Outils de localisation dans l'éditeur : PO/CSV, traductions périmées, polices par langue.
- Galerie, rejouer les scènes, succès, voix.
- Accessibilité : taille de texte, contraste, manette, lecture d'écran quand c'est possible.
- Performance et taille du bundle web.
- Android : `cargo-ndk` / `xbuild`, suspend/resume, tactile, export AAB/APK.
- Éditeur : recherche globale, templates de projet, manuel intégré.
- **Fin :** jeu d'exemple sur les 4 plateformes, traduit en 2 langues dont une CJK.

### M8 : Scripting
- Rhai dans les expressions et les nœuds Script, avec sandbox ; marche aussi sur le web.
- Intégration avec l'état annulable : tout passe par l'API du VM, RNG seedé.
- Plugins Rust côté runtime : handlers des nœuds `Custom` liés au player, API versionnée stabilisée (`orbit_plugin_api`, créée au M3a), exemple de mini-jeu. Export avec plugins. Chargement dynamique éventuel (WASM, ADR 0009).
- DSL texte optionnel avec round-trip, édité dans l'éditeur.
- Éditeur de screens et de menus avancé (layouts personnalisés).
- **Outils MCP :** lecture / édition des nœuds Script et des expressions, diagnostics de la sandbox Rhai. **[edge]** L'IA n'exécute jamais de code hors sandbox ; l'exécution passe par l'API du VM.
- **Fin :** un mini-jeu en Rhai et un nœud en plugin Rust ; la partie Rhai marche aussi sur le web.

### Backlog « plus tard »
- Animation 3D : clips glTF, travelling de caméra, pistes caméra dans la timeline.
- C# (ou plugins WASM), desktop uniquement.
- ~~Import de scripts `.rpy` Ren'Py.~~ **Fait** : crate `orbit_renpy` (éditeur / CLI uniquement, jamais dans le runtime) ; *Fichier > Importer depuis Ren'Py…* et `orbit import-renpy <dossier> <sortie>`. Convertit labels → scènes, menus, conditions, variables, dialogues, show / hide / scene / with, musique / sons, images (convention de nommage Ren'Py), traductions `tl/` ; le reste est listé dans `import-report.md`.
- WebGPU comme backend web optionnel.

---

## 3. Risques
- **Charge de développement du moteur maison / renderer 3D** (nouveau plus gros risque avec l'éditeur) : renderer par couches (`orbit_gfx` → `orbit_render2d` / `orbit_render3d`), wasm en CI dès le M0, 3D reportée au M6 après le MVP, périmètre 3D borné (pas d'animation), briques existantes (`wgpu`, `cosmic-text`, `kira`, `gltf`) plutôt que du code maison.
- **Périmètre de l'éditeur** : UX à deux niveaux, undo/redo dès le départ, no-code comme critère de fin de chaque jalon ; M3 découpé en M3a → M3f.
- **Plugins et dépendances de l'éditeur** : MSRV 1.95 d'egui, `egui-snarl` en 0.x (graphe derrière un trait remplaçable), API de plugin figée trop tôt (`API_VERSION` 0, plugins internes d'abord).
- **Rollback et saves** : état déclaratif, VM déterministe, tests de propriété.
- **Web** : CI wasm dès le M0, budget de taille, lazy loading, profils de qualité WebGL2.
- **Texte CJK/RTL** : validé dès le M1.
- **Dérive de schéma** : versions et migrations dès le M0.
- **Squelettique maison** : périmètre borné (os, skinning, poids), éditeur de rig prévu dès la conception du M5b.
- **Scripting tardif** : invariant « état modifié via l'API, RNG seedé » tenu dès le M0.
- **Sécurité du serveur MCP local et divergence schéma / outils** : désactivé par défaut, localhost seul, jeton, périmètre limité au dossier du projet, destructifs signalés, journal des actions ; accès uniquement via l'API de commandes d'`orbit_project` ; schémas d'outils générés depuis l'IR, test de CI qui compare schémas MCP et types de l'IR ; `orbit_mcp` exclu du player (vérifié en CI).
- **Dérive de périmètre** : critères de fin vérifiables, le reste va au backlog.

---

## 4. MVP (M0 → M4)
- **Inclus :**
  - Dialogues, personnages et expressions, choix, branches, variables.
  - Fonds, musique, transitions.
  - Save/load, rollback, skip, auto, historique.
  - Éditeur no-code extensible par plugins : graphe, scène linéaire, preview, undo/redo, thème ; installeur (M4).
  - Export Windows, Linux et web.
  - **Pilotage par IA via MCP** : v1 headless (M3f) et mode live dans l'éditeur avec preview, capture d'écran et export (M4).
- **Exclus :** animations 2D (tweens, timeline, squelettique), 3D, localisation outillée, scripting, DSL, Android, voix.
- **Critère :** un non-développeur publie en ligne une courte VN, uniquement avec l'interface.

---

## 5. Décisions
1. **Public :** no-code avant tout ; tout est faisable depuis l'éditeur.
2. **Scripting :** reporté au M8 (Rhai + plugins Rust) ; C# au mieux option future desktop. Invariant d'état via l'API et RNG seedé dès le M0.
3. **3D :** scène complète composable dans l'éditeur (glTF, PBR, lumières, ombres, skybox, caméras fixes) ; pas d'animation 3D pour l'instant ; profils web (WebGL2) et desktop.
4. **Animation squelettique 2D :** système maison (os, skinning de mesh 2D, poids, éditeurs de rig et d'animation).
5. **Base technique :** abandon de Bevy ; moteur maison sur wgpu + winit avec bibliothèques briques remplaçables.
6. **Import `.rpy` :** pas pour l'instant (backlog).
7. **Pilotage par IA :** serveur MCP (`orbit_mcp`, SDK `rmcp`) au-dessus d'une API de commandes unique (`orbit_project`, invariant dès le M0) ; headless au M3, live au M4 ; jamais dans le player.

8. **Éditeur extensible :** egui 0.36 intégré à `orbit_platform` (pas `eframe`), outils intégrés = plugins statiques (`orbit_plugin_api` / `orbit_editor_api`), nœuds `Custom` pour étendre l'IR, rendu à la demande ; installeur au M4 (ADR 0009).

## 6. Questions ouvertes
1. Licence du moteur : open source très probable, choix exact à trancher plus tard. En attendant, n'ajouter que des dépendances sous licences permissives (MIT, Apache-2.0, BSD, Zlib) via `cargo deny`.

---

## 7. UI en jeu pilotée par les données et splash screens
- **Statut : runtime fait, éditeur à faire.** Écrans de menu décrits en RON (`orbit_ui::asset`), styles de thème, layout `margin` + `Grid`, `Shell` avec écrans de données (repli sur les écrans Rust), `GamePack` v2 avec bundle UI, export du dossier `ui/`, splash OrbitEngine obligatoire puis splashes du projet. Voir [ADR 0011](adr/0011-ui-data-driven-et-splash.md).
- **Variables sans code (fait, ADR 0011 addendum)** : `VarOp` / `Condition` typés dans `orbit_core` (schéma v7 : `*` et `/` dans les expressions), `Vm::apply_var_op`, `UiAction::VarSet`, `Cond::Vars`, liaisons `Var`, widgets de l'éditeur (inspecteur et éditeur d'interface) avec mode expression. Reste : `SceneGoto`.
- **Personnages animés (fait, ADR 0004 addendum)** : chaque expression est une image fixe ou une animation (sprite sheet / suite d'images, fps, boucle / une fois / aller-retour), schéma v8, rendu par sous-rectangle UV, éditeur dans le panneau des personnages. `Rigged` réservé (avertissement). Reste : voir `TODO_LATER.md`.
- **Reste (éditeur) :** panneau d'édition des écrans / thème / splashes, aperçu dans la preview, activation du bundle par défaut pour un nouveau projet.
