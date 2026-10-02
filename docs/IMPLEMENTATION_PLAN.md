# OrbitEngine : plan d'implémentation (pour agents IA)

Ce document découpe **tout ce qui reste à faire** (après M0–M4, la UI data-driven, le splash, l'éditeur d'interface, les variables sans code v7 et les expressions animées v8) en tâches exécutables par des agents de code. Il complète [ROADMAP.md](ROADMAP.md) (le *quoi* et le *pourquoi*) et [TODO_LATER.md](TODO_LATER.md) (les idées non planifiées) : ici, le *comment* et l'*ordre*.

Sources : ROADMAP §2 (M5–M8), TODO_LATER, ADR 0004 (addenda v6/v7/v8), ADR 0009, ADR 0010, ADR 0011 (et addenda), `git log` jusqu'à `42e3c78`, état du working tree (v7/v8 non commités).

---

## 0. Choix structurants de ce plan (et alternatives écartées)

1. **Phase 0 avant tout le reste, et strictement séquentielle.** Le working tree contient deux changements de schéma (v7, v8) non commités, et `demo.ron` a été modifié par l'utilisateur, ce qui casse des tests dans au moins 8 crates. *Alternative écartée* : avancer M5a en parallèle sur une branche — le moindre changement de schéma (M5a en apporte un, v9) se mélangerait à v7/v8 non validés et rendrait les migrations impossibles à relire. On fige d'abord une base verte et commitée.
2. **Deux sources de test aux rôles distincts.** La démo (`demo.ron`) peut être reconstruite proprement sur le schéma courant (P0-02b) et sert de vitrine **et** de test de bout en bout (chargement, validation, lecture complète, export, i18n). Les tests qui dépendent de détails précis (index de nœuds, nombre de répliques, IDs) visent la fixture `test_minimal.ron` (+ fixtures ciblées) avec ses propres assets, pour ne pas casser à chaque retouche éditoriale de la démo. Décision par test dans P0-03. *Alternative écartée* : tout sur la démo (fragile) ou rien sur la démo (la vitrine pourrait casser sans que la CI le voie).
3. **Un seul numéro de schéma pour l'histoire (`orbit_core::SCHEMA_VERSION`), un par jalon au plus, migrations textuelles additives** (modèle ADR 0004). Chaque tâche qui touche l'IR est marquée « schéma » et doit être *sérialisée* derrière les autres tâches schéma du même jalon. *Alternative écartée* : un numéro par fonctionnalité — trop de migrations triviales, et les agents en parallèle se marcheraient dessus sur `migrate.rs`.
4. **Extraire les types de données UI (`orbit_ui_data`) tôt (UI-01)**, avant les gros ajouts UI. ADR 0011 note le compromis « `orbit_export` dépend des crates de rendu ». Chaque nouvelle fonctionnalité UI aggraverait la dette et le temps de build (critique : PC limité). *Alternative écartée* : le faire « plus tard » — le coût croît avec chaque widget ajouté.
5. **Une seule crate d'animation `orbit_anim` (sans GPU) partagée par tweens, timeline et squelette.** Le temps d'un clip est *déclaratif* (clip + temps local) → rollback/skip/saves gratuits. Le squelette (M5b) consomme la même timeline et le même type `Curve`. *Alternative écartée* : tweens dans `orbit_runtime` et squelette à part — double modèle de temps, deux éditeurs.
6. **3D dans une crate `orbit_render3d` séparée, compositée sous la 2D via render target**, profil WebGL2 conçu en premier (forward, 1 lumière à ombre). *Alternative écartée* : deferred rendering — inutilisable proprement en WebGL2.
7. **Rhai derrière une feature et une crate `orbit_script`**, appelée uniquement via l'API du VM ; jamais d'accès direct au store. Le DSL texte est une vue (parser/printer) sur l'IR, testée par round-trip propriété.

Chaque tâche marquée **ADR d'abord** nécessite un document de conception (`docs/adr/00NN-*.md`) rédigé par un agent `opus` et relu par l'utilisateur *avant* toute implémentation.

---

## 1. Conventions pour les agents

### 1.1 Builds et parallélisme
- Pas de limite de ressources : builds à pleine vitesse, `cargo` en parallèle autorisé.
- **Agents parallèles = un git worktree chacun** (`git worktree add ../orbit-wt-<ID> -b task/<ID>`) : chaque worktree a son propre `target/` (pas de verrou de build partagé) et ses propres fichiers (pas de conflit d'édition). Fusion sur `master` par le coordinateur, une branche à la fois, tests relancés après chaque fusion.
- `cargo test -p <crate>` pendant l'itération ; `cargo test --workspace` avant fusion. `--release` seulement pour le packaging.
- **Ne jamais paralléliser deux tâches sur les mêmes fichiers chauds** : `orbit_core/src/{migrate,story}.rs` (schéma), `orbit_ui/src/asset/migrate.rs` (`ui_version`), `orbit_pak/src/game.rs` (version du pak), `orbit_mcp/src/tools.rs`, locales de l'éditeur (conflits triviaux mais fréquents : fusionner tôt).
- Si l'exécutable de l'éditeur est verrouillé (éditeur ouvert), les tests d'intégration d'`orbit_editor` échouent à l'édition de liens : demander à l'utilisateur de fermer l'éditeur, ne pas tuer de processus.

### 1.2 Données et tests
- La démo (`examples/stories/demo.ron`, `assets/demo/`) **peut être modifiée ou reconstruite** par un agent quand c'est nécessaire (P0-02b, ou pour montrer une nouvelle fonctionnalité) : de préférence via les commandes `orbit_project` / l'éditeur, en la gardant valide sans avertissement. Les tests de détail visent les fixtures (`examples/stories/test_*.ron`, `examples/fixtures/`) ; la démo n'est couverte que par `demo_still_validates` et quelques tests de bout en bout qui n'assertent que des propriétés (pas de comptes exacts).
- Toute tâche livre des tests : unitaires (crates sans GPU), d'intégration (`tests/`), golden/snapshot pour le rendu (readback `orbit_render2d::readback`) quand c'est pertinent.
- Les crates core (`orbit_core`, `orbit_vm`, `orbit_save`, `orbit_i18n`, `orbit_project`, `orbit_anim`, futur `orbit_ui_data`) restent compilables en `wasm32-unknown-unknown` : pas de `std::fs`, `std::thread`, `Instant` hors `orbit_platform`.

### 1.3 Formats
- Changement de format (IR, `UiBundle`, `GamePack`, saves, prefs) ⇒ **ADR ou addendum d'ADR** dans le même commit.
- Changement d'IR ⇒ bump `SCHEMA_VERSION` (`crates/orbit_core/src/migrate.rs`), étape de migration textuelle, test de migration depuis la version précédente, mise à jour des sérialisations « always serialize / strip defaults » (cf. commit `056860f`) et du pak binaire (`orbit_pak`), vérification des saves (`orbit_save`).
- Changement UI ⇒ bump `ui_version` (`orbit_ui/src/asset/migrate.rs`). Changement pak ⇒ bump `GamePack` (`orbit_pak/src/game.rs`) en gardant le décodage des versions précédentes.
- Une seule tâche « schéma » active à la fois.
- **Compatibilité ascendante IMPÉRATIVE** : un projet ou jeu créé avec n'importe quelle version antérieure doit s'ouvrir, se jouer et s'exporter avec la nouvelle (histoires v1→courante, `ui/`, thèmes, préférences, saves et données persistantes des joueurs, paks). Jamais de suppression d'un chemin de migration ou de décodage. Chaque bump ajoute une fixture figée de l'ancienne version (`examples/fixtures/compat/vN/…`) et un test « ancienne version → charge, valide, se joue, sauvegarde/recharge ». Une save faite avec l'ancienne version doit se charger dans la nouvelle (ou être refusée proprement avec message, jamais de crash ni corruption) — refus seulement si inévitable et documenté dans l'ADR.

### 1.4 Éditeur d'abord
- Une fonctionnalité n'est terminée que si elle est **faisable depuis l'éditeur, sans code**, et exposée au MCP (`orbit_mcp/src/tools.rs`) via les commandes d'`orbit_project` (jamais d'écriture directe de fichiers).
- Toute chaîne d'interface de l'éditeur passe par l'i18n (`orbit_editor/src/i18n.rs`) avec entrées **en et fr** (`crates/orbit_editor/locales/{en,fr}/*.ron`). Toute chaîne de jeu passe par la table `ui.*` / `orbit_i18n`.
- Undo/redo via l'API de commandes d'`orbit_project` ; validations dans `validate.rs` → panneau Problèmes (`builtin/problems.rs`).

### 1.5 Modèle recommandé
- `opus` : ADR, architecture, rendu 3D, skinning, rollback, debug difficile, sécurité (sandbox).
- `sonnet` : implémentation standard, éditeur, tests.
- `haiku` : renommages, chaînes i18n, migration mécanique de tests, docs courtes.

### 1.6 Format des tâches
`ID — Titre` · **Taille** S (<½ j agent) / M (1–2 j) / L (3–5 j) / XL (à redécouper) · **Modèle** · **Dépend de** · **//** = parallélisable dans un worktree séparé dès que ses dépendances sont faites. Sans // : la tâche touche un fichier chaud (schéma, format) et passe seule dans sa file. Lots parallèles : §9.

---

## 2. Phase 0 : stabilisation pré-alpha (checklist ordonnée)

P0-01 → P0-05 strictement séquentiel (même working tree non commité), sauf P0-02 (fixture) et P0-02b (démo), parallélisables. Ensuite P0-07, P0-11, P0-12 et UI-02 peuvent tourner en parallèle (worktrees).

**P0-01 — Inventaire du working tree** · S · haiku · dépend : — · //
- `git status`, `git diff --stat` ; classer les fichiers modifiés en : v7 (variables), v8 (expressions animées), UI editor, autres. Produire la liste dans la description du futur commit (pas de fichier).
- Acceptation : chaque fichier modifié est rattaché à un lot.

**P0-02 — Fixture de test autonome** · M · sonnet · dépend : P0-01
- Enrichir `examples/stories/test_minimal.ron` (ou ajouter `test_features.ron`) pour couvrir ce que les tests lisaient dans la démo : personnages + expressions (image fixe **et** `Motion::Animation`), choix, `SetVar`/`If` v7, sauts, musique, transitions, positions libres/miroir/échelle v6.
- Créer `examples/fixtures/assets/` (petites images PNG générées, 1 son court, 1 police libre) ; le fichier d'histoire référence uniquement ces assets.
- Acceptation : `cargo test -p orbit_core` charge et valide la fixture sans avertissement.

**P0-02b — Reconstruire la démo proprement** · L · sonnet (fable pour le texte si besoin) · dépend : P0-01 · // avec P0-02
- Repartir de l'histoire Neo-Kyoto (commit `7123342`) et la reconstruire au schéma courant (v8), de préférence via les commandes `orbit_project` / l'éditeur pour un résultat canonique (IDs ULID, `story.layout`).
- Montrer les nouveautés : splash projet (`ui/`, `SplashDef`) ; écrans data-driven (menu principal, pause, préférences, save/load personnalisés) ; variables sans code (`SetVar`, `If`, choix conditionnés, une valeur affichée par liaison `Var`) ; expressions animées (au moins une sprite sheet et une suite d'images) ; positions libres / miroir / échelle (v6) ; traductions en + fr.
- Nettoyer `assets/demo/` (assets non référencés, casse des noms).
- Acceptation : `orbit validate` sans erreur ni avertissement ; jouable de bout en bout dans l'éditeur et en export web ; c'est la démo livrée par les installeurs (P0-09).

**P0-03 — Répartir les tests entre fixture et démo** · M · sonnet · dépend : P0-02, P0-02b
- Règle par test : **fixture** (majorité) dès que le test dépend d'une structure précise (indices, nombre de nœuds, IDs, contenu d'une réplique, layout du graphe, rollback/seek à un point donné) ; **démo** seulement pour des propriétés globales (charge et valide, se joue jusqu'au bout par choix automatiques, s'exporte, traductions complètes). Chaque test classé porte un commentaire d'une ligne justifiant son choix.
- Fichiers : `orbit_core/tests/demo.rs`, `orbit_vm/tests/demo.rs`, `orbit_vm/src/seek.rs`, `orbit_save/src/tests.rs`, `orbit_cli/tests/cli.rs`, `orbit_i18n/tests/demo.rs`, `orbit_mcp/tests/tools.rs`, `orbit_project/src/graph_layout.rs` (tests), `orbit_pak/src/game.rs` (tests), `orbit_export/src/testing.rs`, `orbit_runtime/src/runtime.rs` (tests), `orbit_editor/tests/{assets,editor,live_mcp,ui_editor}.rs`.
- Les tests migrés vers la fixture : renommer `tests/demo.rs` → `tests/fixture.rs` ; ceux qui restent sur la démo gardent `demo.rs`. Les références non-test (`orbit_player/src/{launch,lib}.rs`, `orbit_editor/src/storage.rs`, `orbit_export/src/project.rs`) restent sur la démo (comportement produit) mais doivent tolérer son évolution.
- Ajouter un test unique `demo_still_validates` (ni erreur ni avertissement, cohérent avec P0-02b) pour détecter une démo cassée sans dépendre de son contenu.
- Acceptation : `grep -r demo.ron crates --include=*.rs` ne renvoie que les usages produit, `demo_still_validates` et les tests de bout en bout justifiés ; `cargo test --workspace` passe.

**P0-04 — Exécuter les tests d'intégration de l'éditeur** · S · sonnet · dépend : P0-03
- Éditeur fermé ; `cargo test -p orbit_editor` ; corriger les régressions v7/v8/UI editor (`tests/scene_editor.rs`, `scene_view.rs`, `ui_editor.rs`).

**P0-05 — Commits du travail en attente** · S · sonnet · dépend : P0-04
- Trois commits logiques : (a) schéma v7 variables sans code, (b) schéma v8 expressions animées, (c) migration des tests vers les fixtures. Vérifier que les ADR (0004 addendum v8, 0011 addendum v7) sont inclus.

**P0-06 — Corrections UI editor rapides** · M · sonnet · dépend : P0-05 (voir UI-02…UI-05 ; seules UI-02 et UI-03 sont bloquantes pour l'alpha).

**P0-07 — CI release Linux** · M · sonnet · dépend : P0-05
- `.github/workflows/ci.yml` : job release sur tag (`v*`) qui construit l'éditeur et le player Linux (AppImage/deb via `packaging/templates`), publie les artefacts. Windows idem si absent.
- Acceptation : un tag de test sur une branche produit les artefacts (vérifié par l'utilisateur ; l'agent ne pousse pas sans accord).

**P0-08 — Signature de code** · M · opus (conception) + utilisateur · dépend : P0-07
- Windows (Authenticode, certificat à acquérir par l'utilisateur : secret CI, jamais dans le dépôt) ; Linux : sommes SHA256 signées (minisign/GPG). Documenter dans ADR 0010 (addendum).
- Risque : coût/délai du certificat ; non bloquant pour une alpha privée.

**P0-09 — Reconstruire et tester les installeurs** · S · utilisateur + sonnet · dépend : P0-05, P0-07
- Builds release (seule exception autorisée), installation sur machine propre, ouverture du projet démo, Play, export web. Checklist manuelle consignée dans `docs/premiers-pas.md` si des étapes changent.

**P0-10 — Push et tag `v0.1.0-alpha`** · S · utilisateur · dépend : P0-09.

**P0-11 — Tests shell du splash** · S · sonnet · dépend : P0-05
- `orbit_runtime/src/shell/tests.rs` : séquence Ready → Splash (moteur puis projet) → menu ; clic pour passer ; splash sans image ; hot reload qui ne rejoue pas le splash.

**P0-12 — Nettoyage** · S · haiku · dépend : P0-05
- Supprimer la chaîne morte `explorer.add_say_tip` (en + fr, et son usage s'il en reste).

---

## 3. UI phases 2–3 et lacunes restantes (préfixe UI)

**UI-01 — Extraire `orbit_ui_data`** · M · sonnet (opus en revue) · dépend : P0-05 · **addendum ADR 0011**
- Déplacer `orbit_ui/src/asset/{screen,widget,action,bundle,migrate,validate,slots}.rs` (types, migration, validation sans rendu) et les types de layout/style sérialisables de `theme.rs` vers une nouvelle crate `crates/orbit_ui_data` (sans wgpu, compilable wasm). `orbit_ui` réexporte.
- `orbit_export` et `orbit_pak`/`orbit_mcp` dépendent de `orbit_ui_data` seulement.
- Acceptation : `cargo tree -p orbit_export` ne contient plus `wgpu` ; tests déplacés passent.
- Risque : cycles avec `orbit_core::varops` (OK : `orbit_ui_data` → `orbit_core`).

**UI-02 — Ctrl+S / Ctrl+Z dans l'éditeur d'interface** · S · sonnet · dépend : P0-05
- `orbit_editor/src/builtin/uied/{mod,model}.rs` : brancher sur l'historique de commandes ; Ctrl+S sauvegarde le document UI courant.
- Tests : `tests/ui_editor.rs` (modèle pur : édition → undo → état initial).

**UI-03 — Invite d'enregistrement au changement de projet** · S · sonnet · dépend : UI-02
- Documents UI modifiés comptés dans le « dirty » global ; dialogue Enregistrer / Ignorer / Annuler (en+fr).

**UI-04 — Hot reload de `ui/` pendant le Play** · M · sonnet · dépend : P0-05
- `orbit_editor/src/preview_host.rs` + `orbit_runtime/src/runtime/embedded.rs` : surveiller `ui/`, revalider, remplacer le bundle du `Shell` sans relancer l'histoire ; bundle invalide → garder l'ancien + problème affiché.

**UI-05 — Aperçu UI avec les traductions de l'histoire** · S · sonnet · dépend : P0-05
- `orbit_ui/src/asset/preview.rs` (`PreviewNav`) : textes `ui.*` résolus avec la table de la langue choisie dans l'éditeur (pas les textes moteur) ; données fictives restent pour sauvegardes.

**UI-06 — Onglet Thème : toutes les sections** · M · sonnet · dépend : UI-01
- Exposer les sections héritées du `Theme` (textbox, choix, quick menu…) comme styles nommés ou inspecteur auto-généré ; migration `ui_version` si les sections deviennent des styles.

**UI-07 — Écran Historique data-driven** · M · sonnet · dépend : UI-01, UI-11
- Widget composite `HistoryLog` (comme `SaveSlots`), `SYSTEM_SCREENS` += `history`, gabarit `assets/ui/default/history.screen.ron`.

**UI-08 — Sélecteur d'image du splash** · S · sonnet · dépend : P0-05
- Utiliser le sélecteur d'assets standard (bibliothèque, filtres, miniatures) dans `uied/props.rs`.

**UI-09 — `UiAction::SceneGoto`** · M · opus · dépend : P0-05 · **addendum ADR 0011**
- `orbit_vm/src/vm.rs` : `Vm::goto_scene(scene_id)` hors nœud — vide la pile d'appels, crée un point de rollback, coupe le futur connu. `orbit_runtime/src/shell.rs` applique l'action.
- Tests VM : rollback après goto, sauvegarde/chargement après goto.

**UI-10 — Dropdown lié à une variable ; avertissements** · S · sonnet · dépend : UI-01
- `BindPath::Var` pour Dropdown (options typées) ; condition « variable comparée à une expression » ; avertissement division par zéro (`EvalError::DivisionByZero` détectable statiquement : diviseur littéral 0 ou variable jamais ≠ 0) dans Problèmes pour `SetVar` en mode expression (`orbit_core/src/validate.rs`).

**UI-11 — Clipping (scissor) dans `orbit_render2d`** · M · sonnet · dépend : P0-05
- Pile de rectangles de scissor dans `batcher.rs` (flush au changement), respect du facteur d'échelle ; puis vraie `ScrollList` (défilement molette/glisser/manette) et popup de Dropdown.
- Tests : readback golden d'une liste tronquée.

**UI-12 — Vraie Grid, 9-slice, polices nommées, sons par style** · L · sonnet · dépend : UI-01, UI-11 · format UI
- `Fill::NineSlice { image, insets }` (sprite découpé en 9 quads dans `orbit_render2d/src/sprite.rs`), `Theme::fonts: BTreeMap<String, FontRef>`, sons `hover/click` par style, layout Grid avec spans.
- Éditeur : éditeur d'insets visuel, liste de polices du projet. `ui_version` +1.

**UI-13 — Valeurs liste/map dans les variables** · L · opus · dépend : P0-05 · **ADR 0016** · schéma
- `orbit_core` `Value::List`, `Value::Map` ; opérations (`Push`, `Remove`, `Contains`, `Len`) dans `VarOp`/expressions ; saves ; affichage éditeur. Prérequis à UI-15.
- Risque : explosion de la taille des snapshots de rollback → partage structurel (`Arc`) ou limites.

**UI-14 — Écrans de gameplay depuis l'histoire (`NodeKind::OpenScreen`) et HUD** · L · opus (ADR) puis sonnet · dépend : UI-09, UI-01 · **ADR 0016** · schéma
- Nœuds `OpenScreen { screen, wait, result, params }` et `CloseScreen` ; `UiAction::Close { result }` ; écrans non bloquants (`pauses_game: false` + `hud: true`) dessinés par-dessus le jeu ; état « écran ouvert » dans le snapshot VM (rollback correct).
- Éditeur : nœud dans le graphe + inspecteur ; gabarits carte, téléphone, inventaire.

**UI-15 — Listes générées depuis une variable** · M · sonnet · dépend : UI-13, UI-11
- Widget `Repeat { source: BindPath, template: WidgetDef }`.

**UI-16 — Transitions d'écran UI** · M · sonnet · dépend : M5A-02 (courbes) · format UI
- `ScreenAsset::transition_in/out` (fade, slide, scale + courbe).

**UI-17 — Outils MCP pour l'UI** · M · sonnet · dépend : UI-01
- `orbit_mcp/src/tools.rs` : lister/lire/créer/modifier écrans et styles via commandes `orbit_project` (ajouter les commandes UI à `orbit_project/src/ext.rs` si absentes).

**UI-18 — `UiAction::Custom` pour plugins** · S · sonnet · dépend : M8-04.

**UI-19 — Splash vidéo** · L · opus · dépend : — · **ADR d'abord** · différé
- Décodeur (desktop : ffmpeg exclu pour licence/taille ; candidats : AV1 via `rav1d`, ou `<video>` HTML sur web) ; probablement backlog.

**UI-20 — Refonte du thème par défaut** · L · sonnet · dépend : UI-12
- Cf. TODO_LATER « Moderniser le thème » : quick menu sans chevauchement, plusieurs thèmes prêts à l'emploi sélectionnables.

**UI-21 — Lacunes animation (avant M5a)** · M · sonnet · dépend : P0-05
- Vue de scène (`builtin/sceneview`) joue l'animation (pas seulement l'image 0) ; frames de taille variable (ancrage par frame) ; saut d'images selon le temps réel.
- **UI-21b — Import GIF/APNG** · M · sonnet : importeur dans `builtin/importers.rs` → suite d'images + fps ; crate `image` (features gif/png).
- **UI-21c — Atlas irrégulier** · M · sonnet · schéma (v9 groupé avec M5a) : `AnimFrames::Atlas { rects }` + éditeur de découpe.

---

## 4. M5a : tweens, courbes, timeline, effets (préfixe M5A)

**M5A-00 — ADR 0012 « Animation déclarative et timeline »** · M · opus · **ADR d'abord** · //
- Contenu : crate `orbit_anim` (sans GPU) ; `Curve` (presets + Bézier cubique) ; `Track<T>` à clés ; `Clip { tracks, duration }` ; état = `(clip_id, start_time)` dans le snapshot VM ; règle rollback/skip → état final ; où vivent les données (IR de scène, `Story::curves` nommées) ; schéma v9 unique pour tout M5a ; impact pak/saves.
- Arbitrer : timeline *par nœud* (bloc de commandes parallèles) vs *par scène* (pistes globales). Recommandation : nœud `Timeline` contenant des pistes, plus commandes rapides `Move/Shake/Zoom/Flash` qui sont du sucre compilé en mini-timeline.

**M5A-01 — Crate `orbit_anim` : courbes et tweens** · M · sonnet · dépend : M5A-00
- `Curve::eval(t)`, presets (linear, ease in/out, back, bounce, elastic), Bézier par Newton+bisection ; `Tween<T: Lerp>` (Vec2, f32, couleur, angle).
- Tests : propriétés (monotonie bornes, t=0/1), mêmes résultats natif/wasm (pas de flottants dépendants de la plateforme : `libm`).

**M5A-02 — Courbes nommées dans l'IR + éditeur de courbes** · M · sonnet · dépend : M5A-01 · schéma v9 (1/3)
- `Story::curves: BTreeMap<String, Curve>` ; transitions existantes (`orbit_render2d/src/transition.rs`) acceptent une courbe.
- Éditeur : widget réutilisable `CurveEditor` dans `orbit_editor_api` (poignées Bézier, presets, aperçu animé) ; panneau Définitions.

**M5A-03 — Commandes Move / Shake / Zoom / Flash / Tint** · M · sonnet · dépend : M5A-02 · schéma v9 (2/3)
- `NodeKind` ou commandes de scène ; runtime (`orbit_runtime/src/stage.rs`) applique les tweens ; skip/rollback → état final.
- Éditeur : inspecteur de nœud avec cible (personnage/fond/caméra), durée, courbe, « attendre la fin ».

**M5A-04 — Nœud Timeline : modèle et runtime** · L · sonnet (opus revue) · dépend : M5A-03 · schéma v9 (3/3)
- Pistes parallèles (propriété × cible), clés, durée, boucles ; évaluation pure `orbit_anim::Clip::sample(t)` ; intégration snapshot VM.
- Tests : rollback au milieu d'une timeline, save/load, seek (`orbit_vm/src/seek.rs`).

**M5A-05 — Éditeur de timeline** · XL→ découpé · sonnet
- **a** panneau dock, règle temporelle, scrubbing avec aperçu dans la vue de scène (M) ; **b** pistes et clés : ajout, glisser, multi-sélection, copier/coller, undo (L) ; **c** interpolation par clé avec `CurveEditor` (M) ; **d** enregistrement « auto-key » en déplaçant dans la vue de scène (M).
- Dépend : M5A-04, UI-21.

**M5A-06 — Personnages en calques** · M · sonnet · dépend : M5A-04 · schéma (v10 si après v9 publié, sinon v9)
- Expression = pile de calques (corps, visage, bouche, yeux clignants animés).

**M5A-07 — Parallax** · M · sonnet · dépend : M5A-03
- Fonds multi-calques avec facteur de profondeur ; caméra 2D (`orbit_render2d/src/camera.rs`) pan/zoom pilotée par timeline.

**M5A-08 — Particules 2D** · L · sonnet (opus pour le budget WebGL2) · dépend : M5A-01
- Émetteurs déterministes sans état (hash `(seed, index)` par particule, pas de RNG du VM ni `rand` libre ; ADR 0012 §6), CPU, instanciés dans le batcher ; presets (pluie, neige, pétales, poussière) ; éditeur avec aperçu.
- [edge] rollback : particules recalculées depuis (seed, temps), pas sauvegardées.

**M5A-09 — MCP animations** · S · sonnet · dépend : M5A-05.

**M5A-10 — Critère de fin M5a** · M · sonnet · dépend : tout M5A
- Fixture `test_anim.ron` créée via commandes `orbit_project` seulement ; tests rollback ; build web (CI) et budget de taille.

---

## 5. M5b : animation squelettique 2D (préfixe M5B)

**M5B-00 — ADR 0013 « Rig 2D maison »** · L · opus · **ADR d'abord** · //
- Modèle : `Rig { bones (parent, rest transform), meshes (sommets, UV, triangles), weights (≤4 os/sommet), slots/draw order }`, fichier `.rig.ron` comme asset (ID stable), `Motion::Rigged { rig, clip }` (la variante réservée v8).
- Skinning : CPU (recommandé pour WebGL2 et quelques dizaines d'os) vs GPU (uniform buffer de matrices, limite d'uniforms WebGL2). Recommandation : CPU d'abord derrière un trait.
- Clips squelettiques = pistes de la timeline M5a (rotation/translation/échelle d'os, slot, déformation).

**M5B-01 — Modèle et évaluation dans `orbit_anim`** · M · sonnet · dépend : M5B-00, M5A-04
- Pose, hiérarchie, matrices monde, skinning linéaire ; tests numériques.

**M5B-02 — Rendu mesh 2D déformé** · M · opus · dépend : M5B-01
- `orbit_render2d` : pipeline mesh texturé indexé (nouveau `mesh.wgsl`), intégration batcher/calques ; golden tests.

**M5B-03 — Éditeur de rig : os** · L · sonnet · dépend : M5B-01
- Panneau dédié (plugin intégré) : création/parentage d'os à la souris, gizmos rotation, hiérarchie.

**M5B-04 — Maillage depuis une image** · L · opus · dépend : M5B-03
- Contour auto (alpha) + triangulation (Delaunay contrainte, crate `spade`) ; édition manuelle des sommets.

**M5B-05 — Peinture des poids** · L · sonnet · dépend : M5B-04
- Pinceau (rayon, force, normaliser), auto-poids par distance, visualisation en dégradé.

**M5B-06 — Clips squelettiques dans la timeline** · M · sonnet · dépend : M5B-05, M5A-05.

**M5B-07 — Runtime, rollback, saves, MCP** · M · sonnet · dépend : M5B-06.

**M5B-08 — Critère de fin M5b** · M · sonnet : personnage riggé dans une fixture, natif + web.

**M5B-09 — Import PSD en calques → rig à régions** · M · sonnet · dépend : M5B-03 · optionnel (ADR 0013 §7 ; pas d'import Spine/Live2D).

Notes ADR 0013 : `Motion::Rigged(RiggedMotion { rig, skin, clip, loop_mode })` en v11, migration v10→v11 retirant l'ancien `Rigged` unitaire (sans effet) ; `.rig.ron` versionné par `rig_version` ; IK 1–2 os optionnelle dans M5B-03/06.

---

## 6. M6 : 3D (préfixe M6)

**M6-00 — ADR 0014 « Renderer 3D et scènes 3D »** · L · opus · **ADR d'abord** · //
- `orbit_scene` (graphe sans GPU : nœuds, transforms, mesh/material/light/camera refs), `orbit_render3d` (forward PBR metallic-roughness), composition 3D → render target → sous la pile 2D existante ; profils qualité (web : 1 lumière à ombre, ≤4 lumières, KTX2 ; desktop : plus) ; format `.scene3d.ron` ; commande de scène « afficher scène 3D + caméra fixe » ; schéma.

**M6-01 — `orbit_scene` (données)** · M · sonnet · dépend : M6-00.
**M6-02 — Import glTF** · M · sonnet · dépend : M6-01 · crate `gltf` ; maillages, matériaux, textures, hiérarchie ; animations ignorées (avertissement).
**M6-03 — `orbit_render3d` : pipeline PBR de base** · L · opus · dépend : M6-01 · golden tests readback.
**M6-04 — Lumières et ombres (shadow map directionnelle, PCF)** · L · opus · dépend : M6-03.
**M6-05 — Skybox / environnement (cubemap, IBL simplifiée)** · M · opus · dépend : M6-03.
**M6-06 — Post-process (tonemapping ACES, bloom, vignette) par profil** · M · sonnet · dépend : M6-03.
**M6-07 — Composition 2D sur 3D et transitions 2D↔3D** · M · opus · dépend : M6-03 · `orbit_runtime/src/stage.rs`.
**M6-08 — Éditeur 3D : viewport, grille, caméra d'édition** · L · sonnet · dépend : M6-03.
**M6-09 — Gizmos (translate/rotate/scale), hiérarchie, inspecteur** · L · sonnet · dépend : M6-08 · crate `transform-gizmo-egui` à évaluer.
**M6-10 — Éditeur de matériaux avec aperçu sphère** · M · sonnet · dépend : M6-08.
**M6-11 — Lumières, skybox, caméras fixes dans l'éditeur ; commandes `ShowScene3d` / `Camera3d` / `HideScene3d` (ADR 0014)** · M · sonnet · dépend : M6-09 · schéma.
**M6-12 — Profils de qualité à l'export + avertissements de validation** · M · sonnet · dépend : M6-06 · `orbit_export`.
**M6-13 — Web : KTX2, budget, test CI wasm** · M · opus · dépend : M6-12.
**M6-14 — MCP 3D + capture de preview** · S · sonnet · dépend : M6-11.
**M6-15 — Critère de fin M6** · M · sonnet.

---

## 7. M7 : polish, localisation, accessibilité, Android (préfixe M7)

**M7-01 — Localisation PO/CSV** · M · sonnet · dépend : P0-05 · `orbit_i18n` : export/import PO et CSV, détection traductions périmées (hash du texte source par ID de chaîne), panneau Traductions dans l'éditeur.
**M7-02 — Polices par langue + fallback CJK/RTL** · M · sonnet · dépend : UI-12 · `orbit_text`.
**M7-03 — Galerie** · M · sonnet · dépend : UI-14 · données persistantes (« déverrouillé ») dans `orbit_save` (portée globale), écran data-driven ; existe partiellement (`orbit_player/src/gallery.rs`) : unifier.
**M7-04 — Rejouer les scènes** · M · opus · dépend : UI-09 · VM en mode replay (variables isolées, retour au menu).
**M7-05 — Succès** · M · sonnet · dépend : UI-13 · définitions dans l'IR (schéma), nœud `Unlock`, écran, hook plateforme (Steam : backlog).
**M7-06 — Voix** · M · sonnet · dépend : P0-05 · voix par réplique (asset), auto-avance sur fin de voix, volume voix, rejouer la voix ; import par lot (nommage par ID de réplique).
**M7-07 — Accessibilité : taille de texte, contraste élevé, réduction des animations** · M · sonnet · dépend : UI-12, M5A-04 · prefs (`orbit_save`), thème « contraste ».
**M7-08 — Manette et navigation clavier/focus dans la UI** · M · sonnet · dépend : UI-11 · `gilrs`, graphe de focus des widgets.
**M7-09 — Lecteur d'écran** · L · opus · **ADR 0017** · `accesskit` (desktop), ARIA live region (web).
**M7-10 — Android : plateforme** · L · opus · **ADR 0017** · `orbit_platform` (android-activity GameActivity, `AAssetManager`, sauvegarde auto au suspend, suspend/resume, tactile, stockage), `cargo-ndk`.
**M7-11 — Android : export APK/AAB** · L · sonnet · dépend : M7-10 · `orbit_export` (player précompilé + `aapt2`/`bundletool`/`apksigner` ; keystore jamais stocké dans le projet), signature (keystore utilisateur), panneau Export.
**M7-12 — Éditeur : templates de projet, manuel intégré** · M · fable (rédaction) + sonnet.
**M7-13 — Critère de fin M7** : jeu d'exemple sur 4 plateformes, 2 langues dont une CJK.

---

## 8. M8 (optionnel) : scripting et plugins (préfixe M8)

**M8-00 — ADR 0015 « Rhai, sandbox, API VM »** · L · opus · **ADR d'abord** · limites (opérations, mémoire, profondeur), API exposée (lecture/écriture variables via VM, RNG seedé), pas d'I/O, déterminisme pour rollback.
**M8-01 — `orbit_script` + nœud `Script` + expressions Rhai** · L · sonnet · dépend : M8-00 · schéma ; natif + wasm.
**M8-02 — Éditeur de script (coloration, diagnostics dans Problèmes)** · M · sonnet.
**M8-03 — MCP scripts (lecture/édition, diagnostics sandbox)** · S · sonnet.
**M8-04 — Plugins Rust runtime : handlers `NodeKind::Custom`, `UiAction::Custom`, `API_VERSION` 1** · L · opus · `orbit_plugin_api`, player et export « avec plugins » (`cargo build` du player).
**M8-05 — Exemple mini-jeu (Rhai) + plugin Rust** · M · sonnet.
**M8-06 — `orbit_dsl` : parser/printer (ADR 0015 §5)** · L · opus · round-trip testé par propriétés (IR → texte → IR identique, IDs ULID conservés).
**M8-07 — Édition DSL dans l'éditeur (vue texte synchronisée)** · L · sonnet.

---
---

## 8b. Reste de TODO_LATER (préfixe TL)

Couverture complète de [TODO_LATER.md](TODO_LATER.md) : variables sans code → UI-09, UI-10 ; personnages animés → UI-21/21b/21c, TL-01, M5B (rigged) ; thème de base et éditeur de thème → UI-06, UI-11 (clipping, mesure du texte), UI-12, UI-20 ; courbes d'easing → M5A-01/02/09 ; extension `.orbit` → TL-02 ; optimisations → TL-03, TL-04 ; bulles → TL-05/06 ; auto-update → TL-07.

**TL-01 — Animations sur les anciens écrans (portraits, UI) ; saut d'images en retard** · M · sonnet · dépend : UI-21 · //
- Portrait de textbox et `Image` des écrans data-driven acceptent `Motion::Animation` (même lecteur que la scène) ; le lecteur saute des images selon le temps réel quand une frame est en retard.

**TL-02 — Extension `.orbit` pour les histoires** · M · sonnet · dépend : P0-05 · // · *validé par l'utilisateur (2026-10-02), à faire avant le tag alpha si possible*
- Les histoires sont enregistrées en `.orbit` (contenu RON inchangé) ; `.ron` reste lu pour la compatibilité, et un projet `.ron` ouvert est proposé à la conversion (renommage, références du manifeste mises à jour).
- Points à toucher : chargement et sauvegarde (`orbit_project`, `orbit_editor` storage / dialogues ouvrir-enregistrer / projets récents / filtres de fichiers), `orbit_cli`, `orbit_export`, `orbit_player` (lancement d'un fichier), `orbit_mcp`, hot reload (`FileWatcher`), modèles de nouveau projet.
- Manifeste : `.orbitproj` ou non → addendum ADR 0004 (décision opus courte avant le code). Traductions, thèmes, `ui/` et préférences restent en `.ron`.
- Installeurs : association de fichier `.orbit` (Windows WiX/NSIS, Linux deb/AppImage `.desktop` + type MIME) avec icône dédiée ; double-clic → ouvre l'éditeur sur le projet.
- Démo et fixtures renommées en `.orbit` (coordonner avec P0-02/P0-02b).
- Acceptation : un projet `.ron` existant s'ouvre et se convertit ; nouveau projet en `.orbit` ; export, CLI et MCP acceptent les deux ; tests de chargement pour les deux extensions ; association testée sur l'installeur Windows.

**TL-03 — Taille du bundle web (suite)** · M · sonnet · dépend : P0-05 · //
- JPEG et polices décodés à la demande, `cosmic-text` sans `swash` inutile, binaryen récent, rapport `twiggy` en CI ; le budget CI ne remonte jamais.

**TL-04 — Profilage CPU/GPU runtime et éditeur** · M · opus · dépend : P0-05 · //
- `puffin` derrière une feature, timestamps GPU wgpu, scénarios de référence ; sortie = tickets de correction chiffrés.

**TL-05 — ADR 0018 « Dialogues en bulles »** · M · opus · **ADR d'abord** · dépend : UI-12, M5A-00 · //
- Mode d'affichage (projet / scène / réplique), ancrage personnage vs position libre, état déclaratif (rollback, skip, historique), queue orientée, empilement vs remplacement, mesure de texte localisé, schéma.

**TL-06 — Bulles : runtime, éditeur, MCP** · L · sonnet · dépend : TL-05, M5A-03
- Rendu `orbit_ui` (style `bubble`), placement par glisser dans la vue de scène, inspecteur de réplique, outils MCP ; tests rollback + machine à écrire + traduction longue.

**TL-07 — Auto-update de l'éditeur** · M · sonnet · dépend : P0-07, P0-08 · //
- `cargo-packager-updater`, manifeste signé publié avec les releases GitHub, vérification au démarrage désactivable ; copie du projet démo dans le dossier utilisateur (jamais écrasé s'il est modifié).


## 9. Graphe de dépendances et ordre recommandé

```
P0-01 → P0-02 → P0-03 → P0-04 → P0-05 ─┬→ P0-07 → P0-08 → P0-09 → P0-10
                                        ├→ P0-11, P0-12, UI-02 → UI-03
                                        └→ UI-01 ─┬→ UI-06, UI-10, UI-12, UI-17
UI-11 ─┬→ UI-12 → UI-20                  │
       └→ UI-07, UI-15                   │
UI-09 → UI-14 (ADR) → M7-03              │
UI-13 (ADR) → UI-15, M7-05               │
M5A-00 (ADR) → M5A-01 → M5A-02 → M5A-03 → M5A-04 → M5A-05 → M5A-10
                          └→ UI-16        └→ M5A-06/07, M5B-01
M5B-00 (ADR) → M5B-01 → 02/03 → 04 → 05 → 06 → 07 → 08
M6-00 (ADR) → M6-01 → 02/03 → 04/05/06 → 07…15
M7 : indépendant de M6 sauf critère de fin ; M7-10 (ADR) → M7-11
M8-00 (ADR) → M8-01… ; M8-04 → UI-18
```

**Lots parallèles (worktrees)** : après P0-05 → {P0-07, P0-11, P0-12, UI-01, UI-02, UI-05, UI-08, UI-11, UI-21, TL-03, TL-04, M7-01, M7-06} ; tous les ADR en même temps ; pendant la chaîne schéma M5A-02→04 → {M5A-07, M5A-08, UI-12, UI-17, TL-01} ; M6-04/05/06 et M6-08 entre eux ; M7-07/08 avec M7-02.

**Ordre conseillé** : Phase 0 → UI-01/02/03/05/08/11 → UI-21 → M5a → UI-12/09/14 → M5b → M7 (localisation, voix, accessibilité peuvent s'intercaler avant M6 car plus de valeur utilisateur) → M6 → M7-10/11 → M8.

**ADR à rédiger d'abord (opus, parallélisables entre eux)** : addendum 0011 pour UI-01/UI-09/UI-12, UI-13 (valeurs liste/map), UI-14 (OpenScreen/HUD), UI-19 (vidéo), M5A-00 (0012), M5B-00 (0013), M6-00 (0014), M7-09 (lecteur d'écran), M7-10 (Android), M8-00 (0015), M8-06 (DSL), TL-05 (bulles).

**Sérialisation des changements de schéma** (seule contrainte forte de parallélisme : un seul agent à la fois sur `orbit_core/src/migrate.rs`) : v9 = M5a (courbes, effets, timeline, atlas UI-21c) ; v10 = UI-13 + UI-14 ; v11 = M5b ; v12 = M6 ; puis M7-05 (+ descriptions d'images, ADR 0017), TL-05 (bulles, ADR 0018), M8-01 (ADR 0015), chacun au prochain numéro libre.

---

## 10. Risques principaux

- **Rollback et état déclaratif** (timeline, particules, OpenScreen, listes, scripts) : chaque ajout à l'état VM doit être couvert par un test « rollback au milieu » et « save/load ».
- **WebGL2** (skinning, 3D, scissor, particules) : profil web conçu en premier, CI wasm à chaque tâche de rendu.
- **Conflits de fusion entre worktrees** : fichiers chauds (§1.1) réservés à une tâche à la fois, fusions fréquentes ; `orbit_ui_data` extrait tôt pour réduire couplage et temps de build.
- **Dérive de schéma** sous agents parallèles : une seule tâche schéma active ; migrations testées depuis v6.
- **Périmètre éditeur** (timeline, rig, 3D sont chacun un mini-logiciel) : tâches XL systématiquement redécoupées, critère « faisable sans code » vérifié en fin de tâche.
