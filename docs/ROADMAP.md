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
  - éditeur : `egui` + `egui-wgpu` + `egui-winit` + `egui_dock` + `egui-snarl` ;
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
- **Éditeur : egui** via `egui-wgpu` et `egui-winit`, avec `egui_dock` pour les panneaux et `egui-snarl` pour le graphe de nœuds.
  - L'éditeur utilise la même `orbit_platform` et le même `orbit_gfx` que le player.
  - La preview est **le vrai runtime, rendu dans une texture wgpu** affichée dans un panneau egui. Le WYSIWYG est exact.
- **UI en jeu : widgets et thèmes maison** (textbox, boutons, menus, sliders, listes), rendus par `orbit_render2d`.
  - Layouts et styles data-driven (RON), entièrement éditables dans l'éditeur de thème.
  - On n'utilise pas egui en jeu : il est inadapté à l'habillage artistique d'un VN.
- **[edge] Texte riche** (machine à écrire, ruby, balises inline, CJK, RTL) : valider `cosmic-text` dès le M1. Prévoir un fallback de polices et des polices réduites pour le web.

### 1.3 Format de données
- **Modèle canonique = IR Rust sérialisée en RON**, un fichier par scène.
  - Chaque nœud a un **ID stable** (ULID) : saves, localisation et diffs en dépendent.
  - Les positions du graphe sont stockées dans un sidecar `.layout`.
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
- Option future desktop uniquement : hosting C# ou plugins WASM (backlog, aucun engagement ; C# ne fonctionne pas sur le web).
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
- `orbit_plugin_api` : traits stables pour les plugins Rust (M8).
- `orbit_player` : binaire du jeu (desktop, web, Android).
- `orbit_editor` : application egui (`egui-wgpu` / `egui-winit`).
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
  - **headless** : `orbit mcp --project <dir>`, transport stdio, sans GPU, travaille sur les fichiers du projet via `orbit_project`.
  - **live** : l'éditeur ouvert expose le MCP en local (HTTP streamable sur 127.0.0.1). Les modifs de l'IA apparaissent en direct, comme un collaborateur, **regroupées en une entrée d'undo par appel d'outil**.
- **Surface exposée :**
  - *Resources* : manifest du projet, liste des scènes / personnages / assets, schéma de l'IR, résultats de lint.
  - *Tools* : lire une scène ; créer / modifier / supprimer nœuds et lignes de dialogue, choix, branches ; personnages et expressions ; importer un asset ; valider ; build / export ; lancer une preview depuis un nœud ; avancer le VM ; lire l'état du VM et les variables ; **capture d'écran de la preview** pour que l'IA « voie » le rendu (mode live).
  - Outils étendus plus tard : animation 2D, rig, scène 3D (M5, M6), scripting (M8).
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
  - le `UiTexts` du `Theme` (`screens.texts`) n'est plus lu : les textes moteur passent par `Texts` (clés `ui.*`) ; à retirer au prochain changement de format du thème ;
  - les textes de l'interface n'existent qu'en anglais et en français (autres langues : anglais), surchargeables par jeu via `RuntimeConfig::ui_text_overrides` mais sans fichier de thème pour l'instant ;
  - sons d'UI : branchés (`Theme::sounds`, chemins relatifs à la racine des assets) mais aucun son par défaut ni test à l'oreille ;
  - la miniature est le décor et les personnages (sans textbox) rendus à 320x180 au moment de la demande de save ; la dernière autosave faite à la fermeture de la fenêtre n'a pas de miniature (pas de relecture GPU possible à ce moment) ;
  - la liste des traductions est un fichier `i18n/index.txt` à tenir à jour (le web ne peut pas lister un dossier) ;
  - sur le web, l'autosave de fermeture n'existe pas (un onglet qui se ferme ne prévient pas l'application) : seules les autosaves de progression et les préférences (écrites au fil de l'eau) sont garanties ;
  - le quick menu chevauche le bas de la textbox ;
  - l'écran de fin renvoie toujours au menu principal (plus de « recommencer » direct).

### M3 : Éditeur v1, sans code
- Application egui sur `egui-wgpu` / `egui-winit` ; dock : explorateur, graphe, éditeur de scène, inspecteur, preview, console de validation.
- **Preview = le runtime rendu dans une texture wgpu**, intégrée à un panneau egui.
- Projets, file watching, import d'assets.
- Graphe macro : scènes, branches, conditions, sauts.
- Éditeur de scène linéaire : dialogues, choix, fonds, sprites, musique, transitions, variables.
- Éditeur de personnages : nom, couleur, sprites, expressions.
- Inspecteur auto-généré depuis l'IR.
- Preview intégrée : « jouer depuis le début » et « jouer depuis ici ».
  - **[edge]** Pour « jouer depuis ici », il faut simuler le VM en headless ou utiliser des presets d'état.
- Undo/redo par commandes. Sauvegarde atomique.
- Éditeur de thème et de widgets en jeu. Lint en direct.
- **MCP v1 headless** (`orbit mcp --project <dir>`, stdio, sans GPU) :
  - resources (manifest, scènes, personnages, assets, schéma de l'IR, lint) ;
  - outils de lecture et d'édition de l'histoire (nœuds, répliques, choix, branches, personnages, expressions, import d'asset) et de validation, tous via les commandes d'`orbit_project` ;
  - schémas d'outils générés depuis l'IR (même source que l'inspecteur) ;
  - testé avec un client MCP réel (ex. Claude Code) ; journal des actions.
  - **[edge]** Une commande refusée par la validation renvoie une erreur structurée, sans modification partielle du projet.
- **Fin :** un non-développeur crée une VN de 5 min uniquement depuis l'interface ; **et** une IA crée via MCP une courte scène avec choix qui passe `orbit validate`.

### M4 : Export multiplateforme (jalon MVP)
- `orbit build` et pak d'assets. Lazy loading HTTP sur le web.
- Export Windows (zip) et Linux (tar.gz).
- Export web : wasm-bindgen, wasm-opt, brotli, loader avec barre de progression, serveur de test local.
- Export lancé depuis l'éditeur (bouton, profils, journal).
- **MCP mode live** : l'éditeur expose le MCP en HTTP streamable sur 127.0.0.1 (désactivé par défaut, jeton, panneau de journal des actions de l'IA).
  - Modifs de l'IA visibles en direct, une entrée d'undo par appel d'outil, file de commandes commune avec l'utilisateur, versions par nœud.
  - Outils supplémentaires : preview depuis un nœud, avancer le VM, lire état et variables, **capture d'écran de la preview**, build / export (destructifs signalés).
  - **[edge]** Éditeur sans GPU disponible (CI) : les outils preview / capture renvoient une erreur explicite, le reste fonctionne.
- Budget de taille wasm mesuré en CI.
- **[edge]** IndexedDB, plein écran, DPI, focus et pause audio.
- Tests end-to-end en CI. Documentation « premiers pas ».
- **Fin :** le jeu du M3 tourne sur une machine propre pour chaque cible ; une IA connectée à l'éditeur ouvert modifie l'histoire en direct, voit la capture de la preview et lance un export via MCP.

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
- Plugins Rust : nœuds personnalisés, API versionnée (`orbit_plugin_api`), exemple de mini-jeu. Export avec plugins.
- DSL texte optionnel avec round-trip, édité dans l'éditeur.
- Éditeur de screens et de menus avancé (layouts personnalisés).
- **Outils MCP :** lecture / édition des nœuds Script et des expressions, diagnostics de la sandbox Rhai. **[edge]** L'IA n'exécute jamais de code hors sandbox ; l'exécution passe par l'API du VM.
- **Fin :** un mini-jeu en Rhai et un nœud en plugin Rust ; la partie Rhai marche aussi sur le web.

### Backlog « plus tard »
- Animation 3D : clips glTF, travelling de caméra, pistes caméra dans la timeline.
- C# (ou plugins WASM), desktop uniquement.
- Import de scripts `.rpy` Ren'Py.
- WebGPU comme backend web optionnel.

---

## 3. Risques
- **Charge de développement du moteur maison / renderer 3D** (nouveau plus gros risque avec l'éditeur) : renderer par couches (`orbit_gfx` → `orbit_render2d` / `orbit_render3d`), wasm en CI dès le M0, 3D reportée au M6 après le MVP, périmètre 3D borné (pas d'animation), briques existantes (`wgpu`, `cosmic-text`, `kira`, `gltf`) plutôt que du code maison.
- **Périmètre de l'éditeur** : UX à deux niveaux, undo/redo dès le départ, no-code comme critère de fin de chaque jalon.
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
  - Éditeur no-code : graphe, scène linéaire, preview, undo/redo, thème.
  - Export Windows, Linux et web.
  - **Pilotage par IA via MCP** : v1 headless (M3) et mode live dans l'éditeur avec preview, capture d'écran et export (M4).
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

## 6. Questions ouvertes
1. Licence du moteur : open source très probable, choix exact à trancher plus tard. En attendant, n'ajouter que des dépendances sous licences permissives (MIT, Apache-2.0, BSD, Zlib) via `cargo deny`.
