# ADR 0009 : Architecture de l'éditeur (M3) : plugins, état et commandes, preview, UX, MCP

Statut : Accepté (implémenté au M3a ; voir « Notes d'implémentation du M3a » en fin de document)

## Contexte

Le jalon M3 livre l'éditeur no-code (ADR 0003 : egui ; ADR 0006 : le player ne dépend jamais de l'éditeur ; ADR 0007 : API de commandes unique). Exigences ajoutées par l'utilisateur :

- tout est faisable depuis l'interface (dialogues, personnages, choix, branches, décors, musique, transitions, variables) ;
- éditeur moderne, léger et rapide : démarrage rapide, quasi aucune consommation au repos ;
- **extensible par plugins**, d'abord pour nous (animation 2D, rig, 3D, bulles de dialogue, voir `docs/TODO_LATER.md`), peut-être des tiers plus tard ;
- installable (peut venir après) ;
- pilotable par une IA via MCP (ROADMAP §1.6).

État du code vérifié : `orbit_project` a déjà `Command` (avec inverse), transactions, `ChangeEvent`, versions par nœud, undo/redo ; `orbit_core` est à l'IR v3 avec `NodeKind` enum fermé (Label, Say, Choice, Jump, SetVar, If, Background, Show, Hide, PlayMusic, StopMusic, PlaySound, Wait, Call, Return, End) ; `orbit_platform::run` boucle en `ControlFlow::Poll` et appelle `App::render` à chaque frame ; `orbit_gfx::Gfx` possède surface, device et queue ; `Renderer2d` sait rendre une `Scene` dans une render target (`render_to_target`) et relire une texture sans bloquer ; `Runtime` charge l'histoire par chemin (`RuntimeConfig::story_path`) et possède son `Renderer2d` ; `Vm::new(story, entry)` et `snapshot` / `restore` existent.

### Versions vérifiées (crates.io, 2 octobre 2026, pas devinées)

| Crate | Version | Dépend de | Licence |
|---|---|---|---|
| `egui`, `egui-wgpu`, `egui-winit`, `egui_extras` | 0.36.2 | `egui-wgpu` : `wgpu ^30.0` ; `egui-winit` et `egui-wgpu` : `winit ^0.30.13` | MIT OR Apache-2.0 |
| `egui_dock` | 0.21.1 | `egui ^0.36` | MIT |
| `egui-snarl` | 0.12.0 (3 releases en 2 mois, 55 k téléchargements) | `egui ^0.36` | MIT OR Apache-2.0 |
| `egui-phosphor` | 0.14.0 | `egui ^0.36` | MIT OR Apache-2.0 (icônes Phosphor : MIT) |
| `schemars` | 1.2.2 | | MIT |
| `rmcp` | 3.5.0 | | Apache-2.0 |
| `wasmtime` (pour plus tard) | 49.0.1 | | Apache-2.0 WITH LLVM-exception |
| `rfd` (dialogues de fichiers natifs, ajouté au M3a) | 0.17.2 | | MIT |

Constats importants :

1. **egui 0.36 est exactement aligné sur notre pile** : `wgpu 30` et `winit 0.30.13` (déjà épinglés). egui 0.35 vise `wgpu 29` et serait incompatible. Aucune adaptation de `wgpu` ou `winit` n'est nécessaire.
2. **egui 0.36 exige Rust 1.95** (MSRV publié) alors que `rust-toolchain.toml` est à 1.94. Il faut relever la toolchain (et `rust-version` du workspace) à 1.95 ; faible risque, à faire en tête de M3a et à valider sur la CI des 3 cibles. Les crates sans egui n'en dépendent pas. **Fait au M3a** (CI à valider sur les runners).
3. `epaint_default_fonts` (embarqué par défaut dans `egui`) a la licence `(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0`, hors de la liste de `deny.toml`. Il est optionnel : on compile egui avec `default-features = false` (sans `default_fonts`) et on fournit nos polices (Inter, OFL, déjà dans `assets/fonts/`, plus la police d'icônes). Évite d'élargir la liste de licences. `cargo deny` reste à valider à l'ajout des dépendances.

## Options et décision

### 1. Intégration egui : boucle propre ou `eframe`

- **`eframe`** : moins de code (boucle, IME, persistance), mais il possède la fenêtre, la boucle et la surface : on ne partage plus `Gfx` / `orbit_platform` avec le player, ce qui contredit ADR 0003 et complique la preview (le `Runtime` doit utiliser le même device wgpu que egui).
- **Intégration directe `egui-winit` + `egui-wgpu` sur `orbit_platform`** (choisie) : un seul device wgpu (celui de `Gfx`) pour egui et pour le runtime ; la texture de la preview est enregistrée telle quelle dans egui (`egui_wgpu::Renderer::register_native_texture`), sans copie ni transfert entre devices.

Conséquences sur `orbit_platform` (changement additif, le player ne change pas) :
- `AppConfig::redraw: Continuous | OnDemand`. En `OnDemand` : `ControlFlow::Wait`, redessin uniquement sur événement fenêtre, `HostRequest::RequestRedraw`, ou échéance `App::next_wake() -> Option<Instant>` (`ControlFlow::WaitUntil`, alimentée par `egui::Context::request_repaint_after`).
- Hook natif optionnel `App::window_event(&mut self, &winit::event::WindowEvent) -> EventResponse` pour donner à `egui-winit` les événements bruts (l'`InputEvent` abstrait du player ne suffit pas : IME, glisser-déposer, presse-papiers). L'éditeur est natif uniquement (Windows, Linux) ; pas d'éditeur web.

### 2. Découpage en crates

- `orbit_plugin_api` : **sans GPU ni egui**, compile en wasm (comme `orbit_core`). Contient le trait `Plugin` (partie « projet ») : manifeste (`id`, version, version d'API requise), types de nœuds personnalisés, validateurs, importeurs d'assets, outils MCP, commandes d'action (sans UI), hooks d'événements projet. Le serveur MCP headless et la CLI ne chargent que cette partie.
- `orbit_editor_api` : dépend d'egui. Trait d'extension `EditorPlugin: Plugin` : panneaux, menus, raccourcis, entrées de palette, inspecteurs, vues de nœuds, nœuds du graphe.
- `orbit_editor` : binaire (`orbit_editor`), hôte egui, registres, dock, thème. Les outils intégrés vivent dans des crates `orbit_editor_*` (ou dans des modules de `orbit_editor` au début), feature-flagués.
- `orbit_mcp` : serveur (M3f headless, M4 live).
- `xtask check-arch` : interdit egui / winit / wgpu à `orbit_plugin_api`, interdit `orbit_editor*` / `orbit_mcp` / egui au player (liste de `arch.rs` à compléter).

La séparation « projet » / « UI » est le point structurant : un plugin d'animation fournit ses nœuds, commandes et outils MCP en `Plugin` (utilisables en headless et par l'IA) et ses panneaux en `EditorPlugin`. Ainsi une fonctionnalité éditable à la main l'est aussi par l'IA (principe directeur).

### 3. Système de plugins

**Décision : plugins Rust compilés statiquement, outils intégrés = plugins (dogfooding), API versionnée dès le départ, chargement dynamique reporté.**

```rust
// orbit_plugin_api
pub trait Plugin: 'static {
    fn manifest(&self) -> PluginManifest;                 // id "orbit.core.scene", version, api_version
    fn register(&self, reg: &mut ProjectRegistry);        // nœuds, validateurs, importeurs, outils MCP, hooks
}
// orbit_editor_api
pub trait EditorPlugin: Plugin {
    fn register_ui(&self, ctx: &mut EditorRegistry);      // panneaux, commandes, menus, raccourcis, inspecteurs
}
```

Registres (clés stables `plugin_id.element`) :
- **Panneaux** : `PanelDef { id, title, icon, default_dock, ui(&mut egui::Ui, &mut EditorContext) }`.
- **Commandes d'éditeur** : `ActionDef { id, label, icon, default_shortcut, enabled(ctx), run(ctx) }`. Palette, menus, raccourcis et barres d'outils sont des vues du même registre ; raccourcis reconfigurables (fichier utilisateur).
- **Inspecteurs** : par type de nœud / de ressource, repli sur l'inspecteur auto-généré depuis le schéma de l'IR.
- **Types de nœuds** : `NodeTypeDef { kind, label, icon, schema, default_data, editor_view, validate, mcp_doc }`.
- **Importeurs** : `AssetImporter { extensions, import(bytes) -> ImportedAsset }`.
- **Outils MCP** : `McpToolDef { name, description, input_schema, destructive, call(&mut ProjectHandle, args) }`.
- **Hooks** : abonnement aux `ChangeEvent` du projet, ouverture / fermeture de projet, avant sauvegarde.

`EditorContext` donne à un plugin : lecture du projet (`&Story`, versions de nœuds), **émission de commandes uniquement** (`ctx.exec(Command)` / `ctx.transaction(label, |tx| ..)`), sélection, préférences du plugin (sandbox de stockage par `plugin_id`), toasts, `ctx.run_action(id)`. Jamais d'accès mutable à la `Story`.

**Étendre l'IR sans casser la sérialisation.** `NodeKind` reste un enum fermé pour les nœuds intégrés (exhaustivité et validation du VM) ; on ajoute **une** variante ouverte (schéma v4, migration v3 → v4 triviale) :

```rust
NodeKind::Custom { plugin_id: String, kind: String, version: u32, data: CustomData }
```

`CustomData` est un petit arbre de valeurs maison (Null, Bool, Int, Float, Str, List, Map, avec identifiants de variantes explicites), et non `ron::Value` : sérialisable en RON lisible comme en `postcard` (qui n'est pas auto-descriptif), diff-friendly. Les `NodeId` / `AssetId` / `StringId` référencés par un nœud custom sont déclarés par le plugin (`NodeTypeDef::references`) pour que la validation (références pendantes), la suppression refusée si référencé et la localisation continuent de fonctionner. Le manifeste de projet liste les `plugins` requis avec leur version ; un projet ouvert sans le plugin garde les nœuds `Custom` intacts (aller-retour sans perte, affichés en lecture seule, avertissement dans la console) : on ne perd jamais de données d'un plugin absent. Les migrations d'un nœud custom sont du ressort du plugin (`version`).

**Exécution côté runtime.** Le VM ne sait rien d'un plugin : sur `Custom`, il émet un événement générique `PresentationEvent::Custom { plugin_id, kind, data }` (non bloquant par défaut, ou bloquant si le `NodeTypeDef` le déclare : attendre une fin) ; son état éventuel est déclaratif dans l'état de scène pour rester compatible rollback / saves. Le player enregistre des `RuntimePlugin` (même crate `orbit_plugin_api`, trait distinct, **sans** egui) liés statiquement (`cargo build` du player avec la feature du plugin ; cohérent avec ROADMAP §1.4 et M8). Un `Custom` sans handler au chargement est une erreur de validation à l'export, un avertissement en preview. L'API de ce trait côté runtime est volontairement réduite en M3 (le crochet existe, les premiers vrais usages arrivent avec M5) : on réserve `Custom` et la validation, pas plus.

**Plugins dynamiques (plus tard, non bloquant).** Comparaison brève :
- *Dylib* (`libloading`) : ABI Rust instable, nécessite `abi_stable` ou une ABI C et du `unsafe` (interdit par `unsafe_code = "forbid"`), pas de sandbox, fragile entre versions de compilateur. Écarté.
- *WASM (wasmtime 49, Apache-2.0)* : sandbox, portable Windows / Linux, API versionnée via WIT / interface minimale, plugins dans n'importe quel langage. Coût : taille du runtime (plusieurs Mo) et UI difficile à exposer. Recommandé **uniquement pour la logique** (importeurs, validateurs, outils MCP, nœuds custom, handlers runtime), les panneaux restant statiques ou décrits de façon déclarative (formulaires générés depuis un schéma).
- *Processus externe* (JSON-RPC sur stdio, comme les serveurs MCP eux-mêmes) : le plus simple pour des outils tiers non critiques en performance ; à garder comme alternative.
- Décision : l'API `orbit_plugin_api` porte un `API_VERSION` ; l'hôte refuse un plugin d'une version incompatible. On n'implémente pas le chargement dynamique avant que des besoins tiers réels existent.

### 4. État et commandes

- **Un seul état de document : `Project`.** Toute modification (éditeur, plugins, MCP, CLI) passe par `Command` / transactions (invariant ADR 0007). Le catalogue de commandes s'étend en M3c (aujourd'hui : InsertNode, RemoveNode, SetSay, SetString) : déplacer / dupliquer / remplacer un nœud, setters par type de nœud, personnages et expressions, variables, scènes, manifeste, assets ; `SetCustomData` pour les nœuds de plugin ; `Command::label()` pour afficher « Annuler : Modifier la réplique ».
- **État d'éditeur séparé** (`EditorState`, pas dans `Project`, jamais annulable par Ctrl+Z) : sélection (`NodeId`, personnage, asset), panneaux ouverts, filtres, zoom du graphe, scroll, état du dock. Les `ChangeEvent` le tiennent cohérent (sélection d'un nœud supprimé vidée ; après undo / redo, la sélection suit le nœud modifié).
- **Disposition (layout)** : dock sérialisé (RON) dans le dossier de configuration utilisateur (disposition globale, `egui_dock` fournit la sérialisation `serde`) ; état propre au projet (onglets, sélection, positions du graphe) dans le sidecar `.layout` (§1.3, hors du RON canonique), `.orbit/workspace.ron` pour l'état de session.
- **Undo/redo global** : une pile par projet (déjà dans `Project`), `Ctrl+Z` / `Ctrl+Shift+Z` / `Ctrl+Y`, menu Édition avec libellé. Une saisie de texte regroupe ses frappes en une entrée (fusion par nœud et par fenêtre de temps via transaction).
- **Sauvegarde** : écriture atomique (fichier temporaire dans le même dossier, `fsync`, renommage ; `std::fs::rename` remplace sur Windows), un fichier par scène ne réécrit que ce qui a changé (`dirty` par version de nœud). Accès disque derrière un trait `ProjectStorage` (§1.6) ; l'implémentation fichiers vit côté éditeur / CLI, pas dans `orbit_project`.
- **Autosave / recovery** : si le projet est modifié depuis la dernière sauvegarde, un instantané est écrit toutes les 30 s (et à l'inactivité) dans `.orbit/recovery/`, jamais par-dessus les fichiers du projet. Au prochain lancement, un marqueur de session non terminée propose « Restaurer / Ignorer ». Modification externe des fichiers (file watching) : si non modifié dans l'éditeur, rechargement ; sinon dialogue de conflit (garder / recharger).

### 5. Preview intégrée

- Le **vrai `Runtime`** rendu dans une render target de `Renderer2d` (taille du canevas virtuel, réduite selon le panneau), affichée dans un panneau egui via `register_native_texture` sur le **même device** que egui (un seul `Gfx`, aucune copie CPU). Préférence à l'enregistrement d'une texture plutôt qu'à un `egui_wgpu::Callback` : plus simple, le runtime fait son rendu avant egui dans la même frame, et la même cible sert à la relecture asynchrone (capture d'écran du MCP, miniatures).
- Changements dans `orbit_runtime` : `Runtime::render_to_target` (pas de présentation sur surface), l'éditeur crée le `Runtime` puis egui emprunte son `Gfx` (`runtime.renderer().gfx()`) : un seul propriétaire ; entrées souris / clavier de la preview transmises par le panneau uniquement quand il a le focus ; **fonts, audio et sauvegardes isolés** (dossier de saves de preview temporaire, jamais les saves du jeu).
- **Chargement depuis la mémoire** : `Runtime::load_story(Arc<Story>)` (aujourd'hui seul le chemin fichier existe). **Hot reload** : à chaque `ChangeEvent` (avec debounce de ~150 ms), l'éditeur rebâtit la `Story` partagée ; le runtime recharge sans passer par le disque et, par défaut, **reprend au nœud courant** si son `NodeId` existe encore (c'est possible parce que les IDs sont stables), sinon au début. Un asset modifié invalide le cache de texture correspondant (dette M1 à lever ici).
- **« Jouer depuis le début »** : démarrage normal. **« Jouer depuis ici »** (nœud sélectionné), par ordre de préférence :
  1. *simulation headless du VM* : recherche bornée (DFS / BFS avec `snapshot` / `restore`, RNG seedé donc déterministe) d'un chemin du début au nœud, en choisissant les options de choix qui mènent au nœud, puis `restore` de l'état obtenu (décor, sprites, musique, variables) dans la preview. Fonctionne sans GPU, testable.
  2. *presets d'état* (variables et décor imposés à la main dans un panneau « État de départ ») quand aucun chemin n'est trouvé (condition jamais vraie) ou qu'on veut forcer un cas ; l'état d'un preset est stocké dans l'espace de travail, pas dans l'histoire.
  Écarté : stocker des snapshots par nœud (invalidés à chaque édition).
- **Rendu à la demande** : la preview n'anime (redessin continu) que si elle est en lecture et visible ; en pause, ou onglet masqué, ou fenêtre en arrière-plan, plus aucune frame (`next_wake` sert uniquement pour la machine à écrire et les transitions en cours).

### 6. UX et visuel

- **Thème maison** (`Visuals` + `Style` egui) clair et sombre, suit le système par défaut, palette sobre à un accent, coins arrondis, espacements en grille de 4 px, états survol / actif / focus nets, **Inter** (déjà dans `assets/fonts/`) en police de base, tailles de texte calibrées ; échelle d'interface (DPI) réglable. Le thème est des données (RON) modifiables pour les tests visuels.
- **Icônes** : `egui-phosphor` 0.14 (MIT OR Apache-2.0 ; les icônes Phosphor sont MIT), chargé comme police d'icônes. Repli possible : Lucide (ISC), à revérifier si Phosphor pose problème. Les icônes de types de nœuds viennent des plugins (clé d'icône du registre).
- **Dock** : `egui_dock` 0.21 (MIT), disposition par défaut « Écriture » (explorateur, scène, preview, inspecteur) et « Graphe », disposition personnalisable et restaurable, panneaux détachables plus tard.
- **Graphe** : `egui-snarl` 0.12 (MIT OR Apache-2.0, dépend de `egui ^0.36`, release régulières). Risque : projet à un mainteneur, API encore en 0.x. Mitigation : le graphe est derrière un trait `GraphView` du panneau (positions dans le sidecar, arêtes calculées depuis l'IR par `NodeKind::targets()`), donc remplaçable par un graphe maison (pan / zoom / arêtes en `egui::Painter`, périmètre modeste pour un graphe de scènes) sans toucher au reste. Étape de validation précoce en M3d (200 nœuds, pan / zoom fluides).
- **Palette de commandes** : `Ctrl+K` / `Ctrl+Shift+P`, recherche floue (`nucleo-matcher` ou implémentation simple, licence à vérifier) sur le registre d'actions + navigation (scènes, nœuds, personnages, assets) ; chaque action montre son raccourci.
- **Recherche globale** : `Ctrl+Shift+F`, sur le texte des répliques, noms, variables, assets, avec résultats cliquables (sélectionne et centre) ; index reconstruit incrémentalement depuis les `ChangeEvent`.
- **Rendu à la demande** : une frame egui n'est dessinée que sur événement ou `request_repaint*` (mécanisme de §1) ; aucune animation permanente hors preview active (curseur clignotant via échéance).

### 7. MCP

- **M3f headless** (`orbit mcp --project <dir>`, stdio) : `rmcp` 3.5 (Apache-2.0, autorisée par `deny.toml`), runtime async réservé à `orbit_mcp` (jamais dans le player ni dans les crates core). Outils de base générés depuis l'IR via **`schemars`** 1.2 (MIT, derive sur les types de l'IR sous feature `schema` d'`orbit_core`, compatible wasm) ; **même source de schéma** que l'inspecteur auto-généré (schéma + indications d'UI), donc pas de divergence IR / éditeur / MCP (test de CI qui compare). Outils des plugins ajoutés via `McpToolDef`.
- **M4 live** : l'éditeur possède le seul `Project`. Un **`CommandQueue`** (canal `mpsc` borné) reçoit les requêtes ; le thread UI les exécute entre deux frames dans une transaction (une entrée d'undo par appel d'outil) puis répond. **L'UI elle-même poste ses commandes dans cette même file** (même chemin d'exécution, donc sérialisation et `expected_version` identiques pour l'humain et l'IA, ROADMAP §1.6). Le serveur (HTTP streamable sur 127.0.0.1, jeton) tourne sur un thread dédié avec son runtime async ; il réveille la boucle UI (`EventLoopProxy` / `request_repaint`) à chaque requête, et les lectures passent par un instantané `Arc<Story>` republié après chaque changement (pas de verrou long). Les captures de la preview utilisent la relecture asynchrone existante ; sans GPU, l'outil renvoie une erreur explicite.
- On prépare M4 dès M3a : l'UI utilise déjà la `CommandQueue` locale, M4 ajoute seulement le transport. Trait `ProjectHost` (exécution directe en headless, via file en live) pour que les outils MCP soient écrits une seule fois.

### 8. Installabilité (ne bloque pas M3)

Windows et Linux : archive portable produite par la CI dès M3 (zip / tar.gz du binaire + `assets/`), puis installeurs en M4 : `cargo-packager` 0.11 (MIT / Apache-2.0 ; MSI via WiX, NSIS, AppImage, deb) ou `cargo-dist` 0.32 (MIT / Apache-2.0) selon la qualité des sorties, le choix se fait au moment de l'implémentation. Auto-update : backlog (non nécessaire pour un moteur à gros jalons). Signature de code Windows : question de budget, hors périmètre.

### 9. Performance (budgets mesurés en CI quand c'est possible)

- Démarrage < 1 s jusqu'à la première frame utile sur le projet démo (fenêtre visible immédiatement, chargement des polices et du projet ensuite, journalisation du temps).
- **0 % CPU et 0 frame au repos** hors preview en lecture : compteur de frames testable (aucune frame en 5 s sans événement) ; pas de redessin continu.
- Mémoire raisonnable : < 300 Mo sur le projet démo avec preview.
- Listes longues (scène de milliers de nœuds, résultats de recherche, explorateur d'assets) **virtualisées** (`ScrollArea::show_rows` / lignes de hauteur connue) ; édition de texte sans reconstruire toute la liste (invalidation par version de nœud).
- Miniatures chargées **paresseusement** (visibles seulement), décodées hors thread UI via `orbit_assets`, cache LRU borné ; validation en direct déboncée et incrémentale par scène.
- Profil de build : `opt-level` 1 pour les dépendances en dev (egui lent en debug), mesure des frames via `puffin` ou compteurs maison si besoin.

## Conséquences

- Positives : un seul `Gfx`, un seul chemin de modification ; les plugins étendent à la fois l'éditeur et l'IA ; les futures fonctionnalités (M5, M6, bulles) sont des plugins qui valident le système ; consommation au repos maîtrisée.
- Coûts : la boucle d'`orbit_platform` est modifiée (additif) ; le `Runtime` doit apprendre à rendre dans une cible et à charger depuis la mémoire ; relèvement de la toolchain à 1.95 ; une nouvelle variante `Custom` dans l'IR (schéma v4) et un contrat de plugin à maintenir ; `egui-snarl` est une dépendance 0.x à surveiller.
- Risques : l'API de plugin s'ossifie trop tôt (on la fait évoluer librement tant qu'il n'y a que des plugins internes, `API_VERSION` 0) ; recherche de chemin « jouer depuis ici » exponentielle sur de gros graphes (bornée, repli sur presets) ; egui n'a pas d'accessibilité complète (AccessKit partiel), non traité en M3.
- À mettre à jour lors de l'implémentation : `deny.toml` (si de nouvelles licences apparaissent), liste des dépendances interdites de `xtask/src/arch.rs`, README (structure), `rust-toolchain.toml`.

## Questions ouvertes

1. ~~Palette de recherche floue : `nucleo-matcher` ou implémentation maison.~~ **Tranché au M3a : maison** (`orbit_editor::fuzzy`, aucune licence ajoutée).
2. Aucun éditeur web : confirmé pour M3 (natif uniquement).
3. Modèle de licence des plugins tiers (à décider avant toute ouverture publique de l'API).

## Notes d'implémentation du M3a

Versions résolues (`Cargo.lock`) : `egui`, `egui-winit`, `egui-wgpu` 0.36.2 ; `egui_dock` 0.21.1 ; `egui-phosphor` 0.14.0 ; `rfd` 0.17.2 ; `wgpu` 30.0.1 et `winit` 0.30.13 inchangés. Toolchain 1.95. `cargo deny check` vert **sans nouvelle exception** (egui sans `default_fonts`, `rfd` MIT).

Ce qui a été fait comme décidé : boucle `orbit_platform` à la demande (`RedrawMode::OnDemand`, `ControlFlow::Wait` / `WaitUntil`, `HostRequest::RequestRedraw`, `App::next_wake`, hook `App::window_event` pour `egui-winit`), `orbit_plugin_api` (sans egui, wasm32, contrôlée par `check-arch`), `orbit_editor_api`, `orbit_editor` (egui-winit + egui-wgpu sur le `Gfx` partagé, pas d'`eframe`), outils intégrés = plugins, plugin d'essai, palette avec recherche floue maison, undo / redo global avec libellés, sauvegarde atomique, autosave et récupération, schéma IR v4 (`NodeKind::Custom`).

Écarts et précisions par rapport au texte ci-dessus :

- **API egui 0.36** : l'API est centrée sur `Ui` (`Context::run_ui`, `Panel::top(..).show(ui, ..)`, `CentralPanel`), pas sur `Context::run` / `TopBottomPanel`. Le code de l'éditeur suit cette API.
- **Polices** : egui sans `default_fonts` ; l'éditeur embarque Inter (`assets/fonts/ui/InterUI-*.ttf`) et la police Phosphor. Inter définit 745 glyphes dans la zone d'usage privé Unicode, exactement là où Phosphor met ses icônes : comme egui prend la première police de la famille qui possède le glyphe, Inter masquait une partie des icônes. Les copies « UI » d'Inter ont donc ces points de code retirés de leur table de caractères (`tools/make_ui_fonts.py`, aucune forme de glyphe modifiée ; OFL 1.1 sans nom réservé, donc modifications permises). Pas de police à chasse fixe : `Monospace` utilise Inter.
- **Rendu** : `Gfx::enable_gamma_view` (opt-in, natif) permet de rendre egui dans une vue non-sRGB de la surface (mélange en espace gamma, comme egui l'attend) ; le player ne l'utilise pas.
- **Rendu à la demande, constats mesurés** : les barres de défilement « flottantes » d'egui et toute animation (`animation_time`) provoquent des rafales de frames ou des redessins sans fin ; le thème utilise des barres solides et `animation_time = 0`. Chaque panneau est monté dans sa propre portée d'identifiants (`push_id`) : deux panneaux avec un `ScrollArea` à identifiant automatique partageaient sinon leur état et se redessinaient indéfiniment. Résultat : 3 frames au démarrage, **0 frame au repos** (testé).
- **Projets** : un projet est un dossier (`orbit.project.ron` : nom, fichier d'histoire, plugins requis ; `story.ron` ; `assets/` ; `.orbit/` pour l'état d'outil, ignoré par git). Le manifeste et le trait `ProjectStorage` sont dans `orbit_project` (sans E/S) ; l'implémentation fichiers (`FsStorage`, écriture atomique, récupération) est dans `orbit_editor`. Une histoire `.ron` seule (`examples/stories/demo.ron`) s'ouvre aussi, sans manifeste ; sa récupération va alors dans le dossier de configuration utilisateur plutôt qu'à côté du fichier.
- **Undo** : `Project` garde un libellé par entrée (`Command::label`, `transaction_labeled`, `undo_label` / `redo_label`) et un compteur `revision`. La saisie de texte est une commande par modification, **fusionnée en une entrée d'annulation par session de saisie** (`Project::execute_merged` / `end_merge`) : l'histoire est à jour à chaque frappe (preview, sauvegarde) et Ctrl+Z annule toute la saisie. La session se termine quand plus aucun champ texte n'a le focus.
- **Autosave** : au plus toutes les 30 s tant que le projet est modifié (pas d'écriture « à l'inactivité » distincte). Le fichier de récupération n'est supprimé que par une sauvegarde, une fermeture propre ou un abandon explicite ; sa présence au lancement suivant déclenche la question « Restaurer / Ignorer ».
- **Dialogues de fichiers** : `rfd` (MIT), appels bloquants entre deux frames.
- **Disposition** : un seul fichier utilisateur (`config.ron` : thème, dock `egui_dock` sérialisé, projets récents, préférences par plugin), réécrit quand un glisser de séparateur est terminé. La disposition propre au projet (`.layout`, `.orbit/workspace.ron`) n'est pas faite.
- **Options de debug** (`orbit-editor --help`) : `--frame-counter`, `--screenshot <png>` (relecture de la frame egui rendue), `--exit-after`, `--open-palette`, `--run-action`, `--select-node`, `--theme`, `--no-config`.

## Notes d'implémentation du M3b

Ce qui a été fait comme décidé : le vrai `Runtime` rendu dans une render target de `Renderer2d`, enregistrée dans egui (`register_native_texture`) sur le **même device** (aucune copie CPU) ; `Runtime::load_story(Arc<Story>, racine d'assets)` ; entrées routées au jeu seulement quand le panneau a le focus ; saves de la preview en mémoire ; hot reload depuis l'état mémoire avec reprise au nœud courant ; « jouer depuis ici » par recherche de chemin bornée sur le VM headless, repli sur l'état par défaut.

Écarts et précisions :

- **Propriété du `Gfx`** : le `Runtime` possède le `Renderer2d` donc le `Gfx` (surface comprise) ; l'éditeur le lui confie (`PreviewHost::new(gfx)`) et egui l'emprunte (`runtime.renderer().gfx()`), comme prévu. Le `Runtime` existe dès le démarrage de l'éditeur (arrêté et vide), même sans projet.
- **Séparation panneau / hôte** : le panneau Preview est un plugin egui sans GPU ; il parle à l'hôte par `EditorState::preview` (`PreviewState` : commandes et entrées vers l'hôte ; statut, texture, nœud courant et erreur vers le panneau). L'API de plugin n'expose donc ni wgpu ni le runtime. L'hôte (`orbit_editor::preview_host`) tourne entre le passage de l'UI et la peinture d'egui, dans la même frame : la cible est à jour avant qu'egui la peigne. La texture d'egui garde le même identifiant quand la cible change de taille (`update_egui_texture_from_wgpu_texture`), la cible est un multiple de 64 px de large avec hystérésis.
- **Texture en espace gamma** : egui peint dans une vue non sRGB de la surface et attend des textures non sRGB ; la cible (sRGB, comme pour les transitions) est donc créée avec une vue `Rgba8Unorm` supplémentaire (`Renderer2d::enable_target_gamma_views`, natif uniquement ; sans effet sur le web).
- **Rendu dans une cible** : `Runtime` dessine via un `Output` (surface ou cible) ; `Renderer2d` gagne `render_to_target_over`, `draw_transition_to_target` et `set_pixel_ratio_override` (le texte est rasterisé pour la taille de la cible, pas celle de la fenêtre). Le chemin surface du player est inchangé.
- **Mode embarqué** (`Runtime::new_embedded`) : polices fournies par l'hôte (Inter embarquée dans l'éditeur, pas de dossier moteur), pas d'écran « cliquer pour démarrer » ni de menu principal (la fin de l'histoire ou « menu principal » relance l'histoire), `MemoryStorage` pour les saves et les préférences, rien de persistant entre deux parties (la preview est déterministe), autosave et miniatures coupées, quitter / plein écran retirés, saut qui passe tout (aucune réplique n'est « déjà lue »), indicateur de la textbox fixe (`TextBoxTheme::indicator_blink`, nouveau champ du thème, vrai par défaut). L'audio démarre au premier « lecture » ; il est coupé (volume maître) pour la pause, le mute et la perte de focus de la fenêtre ; la même piste de musique n'est pas relancée par un rechargement.
- **Rendu à la demande** : le runtime n'a pas de `next_wake` : il expose `is_animating()` (machine à écrire, pause, transition, saut, auto, toast, assets en cours de chargement) et `take_dirty()` (une entrée, un changement d'état, la fin d'un mouvement). L'hôte tourne à ~80 Hz tant que `is_animating()` ; sinon plus aucune frame (le temps ne s'écoule pas pendant l'inactivité : le premier `dt` après un repos est nul). Pas de frame non plus si le panneau est masqué ou la fenêtre sans focus. Mesuré : 0 frame en 7 s de jeu en attente d'un clic.
- **Hot reload** : l'hôte compare `(génération du projet, révision)` ; un changement arme un anti-rebond (100 ms de calme, au plus 250 ms depuis la première modification) ; `Runtime::load_story` valide l'histoire (refus lisible, la dernière version valide continue), charge les assets nouveaux (en asynchrone, l'ancien jeu reste affiché), puis remplace le `Shell` et reprend (`orbit_runtime::preview::plan_resume`) : l'état du VM est restauré sur la nouvelle histoire, **la ligne affichée est ré-exécutée** (son texte, son locuteur ou ses options ont pu changer) et montrée en entier, sans rejouer la machine à écrire. Nœud disparu : label le plus proche avant lui dans sa scène, sinon début de la scène, sinon début de l'histoire, avec un avertissement dans la console. Un asset modifié sur disque (date de modification) est oublié et rechargé (dette M1 levée, sans surveillance de fichiers).
- **Jouer depuis ici** : `orbit_vm::seek` (VM, sans GPU, compile en wasm). `Vm::step_to(nœud)` s'arrête juste avant d'exécuter le nœud cible, ce qui permet de viser n'importe quel nœud (pas seulement une réplique) ; recherche en largeur sur les points de choix (le chemin avec le moins de décisions, options essayées dans l'ordre : déterministe), mémoire des points de choix déjà vus (même nœud, variables et pile d'appels), bornes (4 000 points de choix, 200 000 pas). Le résultat est un `VmState` plus les variables persistantes et les lignes lues, restauré dans un VM ; `shown` est effacé pour que le nœud cible s'exécute tout de suite. Le RNG est celui de `Vm::from_start` (graine fixe) : le résultat égale celui d'une lecture complète (test). Repli (inatteignable ou bornes atteintes) : VM démarrant au nœud avec l'état par défaut et un avertissement. Le panneau de presets d'état n'est pas fait (dette).
- **Capture** : `PreviewHost::start_capture` / `write_capture` relisent la cible (relecture asynchrone existante) ; `--preview-capture <png>` s'en sert. Elle n'est pas encore exposée à l'utilisateur ni au MCP (M4).
- **UI** : nouvelle zone de dock `DockSlot::RightTop` (la preview au-dessus de l'inspecteur), action et état « plein panneau » (`EditorRequest::ToggleMaximizePanel`, `EditorState::maximized_panel`), raccourcis F6 (lecture / pause), Maj+F6 (arrêt), Ctrl+F6 (recommencer), F7 (jouer depuis ici), le jeu recevant ses propres touches quand le panneau a le focus (une pression en dehors du panneau rend le clavier à l'éditeur). L'explorateur marque la ligne en cours de lecture et propose « jouer depuis ici » sur la ligne sélectionnée (bouton et menu contextuel).
- **Options de debug ajoutées** (`orbit-editor --help`) : `--play-preview`, `--preview-skip`, `--play-from-selection`, `--edit-text <n>=<texte>` et `--edit-delay`, `--preview-capture <png>`, `--always-active`.

## Notes d'implémentation du M3f

- **`rmcp` 3.5.0** vérifié (Apache-2.0, `cargo deny check licenses` sans nouvelle exception) avec les features `server` et `transport-io` seulement ; runtime `tokio` à thread unique, réservé à `orbit_mcp` et à `orbit_cli`. Le SDK 3.5 implémente aussi la révision de spec 2026-07-28 ; la négociation reste compatible avec 2025-06-18 (testée).
- **Le `Project` vit sur un thread dédié** : il n'est pas `Send` (registre de plugins, écouteurs) ; le handler asynchrone envoie des requêtes à ce thread par un canal, ce qui sérialise les appels (même modèle que la file de commandes du mode live). Le trait `ProjectHost` (liste / appel d'outils, ressources) est ce que le transport consomme ; `HeadlessHost` l'implémente par exécution directe.
- **Commandes ajoutées à `orbit_project`** (fichier `ext.rs`, sans toucher aux commandes existantes) : `InsertScene`, `RemoveScene`, `RenameScene`, `MoveNode`, `SetNodeKind`, `SetCharacter`, `SetVariable`, `SetAsset`, et `Project::group` (fusionne les entrées d'undo d'un appel, annule tout en cas d'échec). `MoveNode` et `SetNodeKind` portent `expected_version`.
- **Schémas** : `orbit_core` gagne la feature `schema` (`schemars` 1.2, MIT) qui dérive `JsonSchema` sur tous les types de l'IR ; les ids sont des chaînes ULID dans le schéma, comme dans le RON. Les schémas d'entrée des outils sont générés des types d'arguments d'`orbit_mcp` (sous-schémas en ligne, champs inconnus refusés). Le test `tests/schema.rs` compare la liste des `NodeKind` du schéma de l'IR à la table `NODE_KIND_TOOLS` et les champs de chaque variante à ceux de `add_command` : ajouter une variante à l'IR sans outil MCP casse la CI. Les conditions / valeurs sont passées en texte (langage d'expressions) plutôt qu'en AST JSON : plus simple et plus fiable pour une IA, et le même `Expr::parse` que le reste du moteur.
- **Bac à sable** : `Sandbox` (`orbit_mcp::sandbox`) canonicalise, refuse `..`, résout les liens symboliques et refuse les liens pendants ; `FsStorage` (écriture atomique) l'utilise pour `ProjectStorage`.

## Notes d'implémentation du M3c

- **Opérations composées** : `orbit_project::ops` assemble les commandes en gestes de l'utilisateur (une transaction étiquetée = une étape d'annulation, tout ou rien) : insertion de brouillons (`NodeDraft` : nœud + textes), suppression (refus si un nœud extérieur à la sélection cible encore un des nœuds ; les textes disparaissent avec les nœuds), déplacement par bloc ancré sur le nœud suivant, copier / coller (`NodeClip` porte les textes ; nouveaux ids de nœuds et de textes, cibles internes remappées, cibles externes conservées), scènes, personnages, expressions, variables (renommage = déclaration + tous les usages, ou refus). L'éditeur y accède par `EditorContext::ops`, qui ne donne pas d'accès mutable à l'histoire. Commandes ajoutées : `MoveScene`, `SetPositions` (les autres viennent de `ext.rs`, M3f).
- **Éditeur de scène** (plugin `orbit.scene`) : liste virtualisée à hauteurs de ligne variables (préfixes cumulés mis en cache par révision, scène et ligne en édition), lignes peintes directement, toute modification différée en `RowAction` appliquée après le dessin. Multi-sélection dans `EditorState::node_selection`, presse-papiers interne dans `EditorState::clipboard`. Les frappes sont des commandes fusionnées (`execute_merged`) ; la fusion ne se termine que sans champ actif **et** sans bouton enfoncé (glisser une valeur = une étape).
- **Inspecteur** : un `InspectorDef` par type intégré, tous faits des champs de `orbit_editor_api::fields` (les widgets lisent le story et ne rendent une valeur que si l'utilisateur l'a changée ; l'appelant la transforme en `SetNodeKind`). Les plugins utilisent le même registre ; sans inspecteur propre, leurs `data` sont éditées génériquement. Les expressions passent par `Expr::parse` avec erreur en ligne et vérification de type ; un éditeur simple (variable, test, valeur) couvre les cas courants.
- **Diagnostics** : `validate` est relancée par `EditorCore` à chaque révision et regroupée par nœud (`EditorState::diagnostics`, messages français) pour les pastilles de la liste et l'inspecteur.
- **Champ IR** : `Character::default_position` (optionnel, `skip_serializing_if`) pré-remplit les nouveaux `Show` ; pas de changement de version de schéma.
- **Performance** : 5007 nœuds, défilement continu en release : 0,74 ms de CPU par frame en moyenne. Générateur : `orbit-editor --generate-story <n> <fichier.ron>` ; mesure : `--autoscroll <px> --exit-after <s>`.

## Notes d'implémentation du M3e

- **Import** (`orbit_editor::assets`, sans egui) : les importeurs sont ceux de `orbit_plugin_api` (`AssetImporter`, enregistrés par le plugin intégré `orbit.importers` comme n'importe quel plugin : PNG / JPEG par `image`, WAV / OGG par `orbit_assets::decode_audio`). Un importeur **décode** le contenu : refus d'un faux PNG, d'un fichier tronqué, d'une image de plus de 16 384 px ; un son de 20 s ou plus devient une musique. Le fichier est copié (écriture atomique) sous `assets/images`, `assets/music` ou `assets/sounds` ; le nom est nettoyé (lettres, chiffres, `-`, `_`) et rendu unique **sans tenir compte de la casse** (Windows fusionnerait `A.png` et `a.png`). Toutes les déclarations d'un geste (plusieurs fichiers, un dossier) sont **une transaction** ; si elle échoue, les copies sont retirées. Les écritures passent par `orbit_project::sandbox::Sandbox` (déplacé depuis `orbit_mcp`, que `orbit_mcp::sandbox` ré-exporte) : destinations confinées au projet, liens résolus ; les sources, elles, peuvent être n'importe où (on les lit).
- **Renommer / déplacer** : le manifeste d'abord (`ops::set_asset_path` : chemin valide, collision insensible à la casse refusée), puis le fichier ; le **journal des déplacements** (`MoveJournal`, partagé entre la bibliothèque et le noyau) permet à l'annulation et au rétablissement de remettre le fichier là où dit le manifeste (`reconcile_moves`, appelée sur les événements `Undone` / `Redone`) : l'annulation d'une commande ne sait pas déplacer un fichier, l'éditeur le fait pour elle. Retirer une ressource passe par `ops::remove_asset` (message français) et, en dernier recours, par `Command::SetAsset` qui refuse désormais aussi les expressions de personnage (`AssetInUseByCharacter`, `orbit_project::asset_users` donne tous les usages).
- **Lint** : `lint_asset_files` compare chaque segment de chemin du manifeste à la **liste du dossier**, octet pour octet : le résultat ne dépend donc pas de la casse du système (Windows répondrait « existe » pour `Bg.png` quand le fichier est `bg.png`). Résultats : fichier introuvable (erreur), écart de casse et deux chemins qui ne diffèrent que par la casse (avertissements).
- **Miniatures** (`orbit_editor::thumbs` + `builtin::thumbs`) : 2 fils de travail, pile (le plus récemment demandé d'abord : les lignes qu'on vient de faire défiler passent avant celles qu'on a laissées), dédoublonnage par clé (chemin, taille, date), cache disque `.orbit/thumbs/<fnv1a>.png|.txt` borné à 3000 fichiers (élagage au démarrage), cache de textures egui borné (400, retombe à 300, le moins récemment dessiné d'abord). Les fils réveillent la boucle par un `Waker` (`EditorState::waker`), sans polling.
- **`orbit_platform::Waker`** : nouvelle méthode `App::attach_waker`, appelée une fois à l'installation de l'application, qui donne un handle `Send` pour réveiller la boucle (un `UserEvent::Wake` sur le proxy winit). Sur le natif, `UserEvent` ne contient que ce variant (l'application n'est renvoyée par le proxy que sur le web). Les fils (miniatures, surveillance, validation, décodage audio de l'écoute) l'utilisent : l'éditeur reste à 0 frame au repos, sans minuterie.
- **Surveillance** (`orbit_editor::watch`) : fil de sondage (date + taille, 500 ms) sur le fichier d'histoire et `assets/`, la ligne de base prise à l'appel de `start` (pas dans le fil : un fichier écrit juste après serait raté). Un changement n'est reporté que s'il est **stable sur deux balayages** (`Settle`) ; l'écriture de l'éditeur lui-même est déclarée (`accept_story_now`). On n'a pas pris `notify` (nouvelle dépendance, et un sondage de 500 ms suffit à l'échelle d'un projet de VN). Histoire modifiée dehors : si le contenu relu égale celui de l'éditeur, rien ; sinon un dialogue propose de recharger (`Dialog::ExternalChange { conflict }`).
- **Validation en direct** (`orbit_editor::diagnostics::Live`) : `Diagnostics` porte maintenant `problems: Vec<Problem>` (niveau, message français, `Location` : nœud, scène, personnage, variable, asset), triés erreurs d'abord puis dans l'ordre de lecture. Jusqu'à 1500 nœuds la validation est faite sur place (< 1 ms) ; au-delà, **bord montant** (la première modification après une pause part aussitôt) puis au plus une validation toutes les 80 ms, sur un fil qui reçoit un clone de l'histoire et ne garde que la plus récente demande ; les résultats d'une révision périmée sont jetés. Les validateurs de plugins et les plugins absents sont calculés côté interface (le registre n'est pas `Send`) et passés au fil. Les problèmes de fichiers (manquant, casse) sont recalculés quand le manifeste ou le disque change (`mark_assets_dirty`, jamais par frame).
- **Révélation** : `EditorContext::reveal(Location)` sélectionne un nœud ou une scène (l'explorateur défile et déplie, l'éditeur de scène suit, l'inspecteur affiche) ou laisse un `pending_reveal` que le panneau propriétaire prend (personnages : sélection ; variables : cadre mis en évidence ; assets : sélection dans la bibliothèque) ; `builtin::reveal::reveal` y ajoute l'ouverture du panneau. Problèmes, recherche, palette, boutons « utilisée par » de la bibliothèque : un seul chemin.
- **Recherche** (`orbit_editor::search`) : index plat (texte, forme repliée en minuscules sans accents, contexte, `Location`) reconstruit par révision à la demande (3,6 ms pour 10 000 nœuds en release) ; une requête = mots qui doivent tous apparaître (sous-chaîne repliée) ; les noms (personnages, variables, étiquettes, scènes, assets) acceptent aussi une correspondance floue (`fuzzy_match`) ; score : texte exact, début, début de mot, part du texte ; 60 résultats par groupe. L'index est partagé entre le panneau et la palette par `builtin::Shared` (`Rc<RefCell<…>>` : tout vit sur le fil d'interface).
- **Palette** : `navigation` ajoute à la recherche d'actions des entrées de navigation (`PaletteItem::goto`) ; les correspondances d'actions éparpillées (ni suite contiguë ni débuts de mots, `bob` dans « Basculer le thème clair / sombre ») passent après celles du projet (`merge`).
- **Glisser-déposer** : `orbit_editor_api::fields::AssetDrag` est la charge utile (`egui::DragAndDrop`) ; `asset_picker` (et donc tous les champs ressource, dont les expressions de personnage) la reçoit, ainsi que les lignes Décor / Musique / Son de l'éditeur de scène, uniquement pour le bon type. Les fichiers déposés depuis l'OS arrivent par `raw.dropped_files` d'`egui-winit` (`DroppedFile`).
- **Rejeté / reporté** : éditeur de thème de l'UI en jeu (backlog), type d'asset police (l'IR n'en a pas), `notify`, index et validation incrémentaux nœud par nœud (non nécessaires aux budgets mesurés).
