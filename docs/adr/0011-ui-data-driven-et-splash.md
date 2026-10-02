# ADR 0011 : UI en jeu pilotée par les données, et splash screens

Statut : accepté (partie runtime faite ; l'éditeur de menus vient ensuite).

## Contexte

Les menus (principal, pause, préférences) étaient construits en Rust (`orbit_ui::screens`). Un auteur no-code ne pouvait ni les réagencer ni en ajouter. On veut des écrans décrits en données (RON), éditables plus tard dans l'éditeur, sans casser les jeux existants. Deuxième besoin : un splash OrbitEngine **obligatoire** au lancement, puis des splashes propres au projet.

## Décisions

### 1. Un écran = un document RON (`orbit_ui::asset`)

* `ScreenAsset { ui_version, id, modal, pauses_game, dim_background, on_cancel, root }` ; `WidgetDef { id, kind, style, layout, visible, enabled, children, overrides }`.
* `WidgetKind` : Panel, Stack (vertical / horizontal / superposé), Grid, Button, Label, Image, Slider, Toggle, Dropdown (= sélecteur pour l'instant), ScrollList.
* Textes : `TextRef::{Key, Literal, Bind}` (clés `ui.*` de la table d'interface, donc localisés). `visible` / `enabled` sont des conditions typées (`Cond` : `CanContinue`, `CanSave`, `ShowQuit`, `InGame`, `Not`).
* Actions typées (`UiAction`) au lieu de chaînes : `GameNew`, `Open`, `Replace`, `Close`, `CloseAll`, `QuickSave`, `PlaySound`, `Custom { name, args }`... `SceneGoto` et `VarSet` sont déclarées mais sans effet (avertissement de validation) ; tous deux pris en charge depuis (addenda ci-dessous). Liaisons (`BindPath`) : seul `Setting(SettingKey)` fonctionne ; `Var` est réservé.
* Versionné par `ui_version` ; `migrate.rs` suit le schéma de `orbit_core::migrate` (étapes sur le texte RON). `validate.rs` renvoie des `Issue` (erreur / avertissement) : ids dupliqués, bornes de slider, colonnes de grille, liaisons incompatibles, styles inconnus, profondeur.
* `DataScreen` implémente le trait `Screen` : il reçoit un `DataContext` (préférences, droit de continuer / sauvegarder...) et émet les `UiEvent` existants plus `OpenScreen`, `ReplaceScreen`, `CloseAll`, `PlaySound`, `Custom`.

### 2. Layout et styles

* Les types de layout (`Size`, `Dir`, `Align`, `Anchor`, `PanelKind`) sont sérialisables. Ajouts : `margin` (marge extérieure) et `Dir::Grid(colonnes)`.
* `Theme::styles: BTreeMap<String, Style>` : police, taille, couleurs par état (normal / survol / appui / désactivé), `Fill::Color`, bordure, padding, sons ; `extends` sur **un seul niveau**. `resolve_style` est pure. Les `overrides` d'un widget deviennent un style local qui étend le style nommé. `UiTexts` (mort) est supprimé.

### 3. Intégration dans le `Shell`

* `Overlay::Custom(i)`. Les écrans de données s'ouvrent par id ; `main_menu`, `pause` et `preferences` remplacent les écrans Rust s'ils sont fournis (par id, ou via `system_screens`). **Les écrans Rust restent le repli** : un projet sans bundle ne change pas.
* Le jeu s'arrête selon `pauses_game` de l'écran du dessus (les écrans Rust le font toujours). Un écran `modal` est dessiné au-dessus de l'écran du dessous ; un écran ni `modal` ni `pauses_game` masque le jeu (comme les écrans plein cadre).
* Source de données : les réglages du joueur (volumes, vitesse du texte, délai auto, mode de saut, plein écran, langue) via `PrefsView`.

### 4. Chargement et export

* `UiBundle { theme, screens, system_screens, splashes }` ; `UiBundle::builtin()` reprend `assets/ui/default/*.ron` (incluses par `include_str!`) : point de départ d'un projet, **pas** activé par défaut.
* Ordre de résolution du runtime : `RuntimeConfig::ui_bundle` > `ui_dir` (dossier du projet, natif) > bundle du `GamePack`. Un bundle avec une erreur de validation est refusé (écrans Rust utilisés).
* `GamePack` passe en **version 2** : champ `ui: Option<Vec<u8>>` (texte RON d'un `UiBundle`, opaque pour `orbit_pak`, qui reste sans dépendance de rendu). Le décodage v1 est conservé (struct dédiée : postcard n'est pas auto-descriptif).
* `orbit_export` lit le dossier `ui/` du projet, le valide (erreur bloquante) et ajoute les images de splash au pak. Compromis : `orbit_export` dépend maintenant d'`orbit_ui` (donc des crates de rendu à la compilation). Extraire les types de données dans un crate sans rendu reste à faire.

### 5. Splash screens

* Séquence = **splash OrbitEngine** puis splashes du projet (`SplashDef`) puis menu principal. `SplashState` (`orbit_runtime/src/splash.rs`, sans GPU, testé) est le seul moyen de construire la séquence : le splash moteur y est codé en dur (logo `assets/brand/logo-horizontal-light.png` embarqué par `include_bytes!`, 3 s, passable après 1,5 s) ; aucun champ de config, fichier de projet ou entrée de pak ne peut le retirer, le raccourcir ou le réordonner. Les valeurs d'un splash de projet sont assainies (durée bornée, fondus, NaN).
* `Phase::Splash` se place entre `Ready` (le clic de départ, nécessaire pour débloquer l'audio web) et `Running`. Seuls le runtime embarqué de l'éditeur (`runtime/embedded.rs`, qui passe directement en `Running`) et le redémarrage par hot reload dans une session déjà lancée l'évitent ; rien n'y mène depuis `orbit_player`.
* Images seulement (pas de décodeur vidéo) : la variante `SplashContent::Video` est réservée et ignorée.

## Addendum : variables sans code (schéma v7)

* **Modèle partagé** (`orbit_core::varops`) : `VarOp { var, op, value }` (`OpKind` : Set, Add, Sub, Mul, Div, Toggle ; `Add` sur un texte = ajouter à la fin) avec `Operand::{Lit, Var}` ; `Condition::{Cmp { var, cmp, value }, All, Any, Expr}`. Les opérations proposées dépendent du type (`OpKind::for_type`, `Cmp::for_type` : un booléen n'a que « est vrai / faux »). Ce sont des **vues** sur le langage d'expressions : `to_expr` / `from_expr` (reconnaissance de la forme), si bien que les nœuds `SetVar` / `If` et les conditions de choix **gardent leur format** et que les anciennes expressions s'affichent en visuel quand elles ont la bonne forme. Le reste reste une expression (« mode expression »).
* **Schéma v7** : le langage d'expressions gagne `Mul` et `Div` (`*`, `/` dans le texte ; division entière sur deux entiers). Migration v6 → v7 : simple changement de numéro (additif). Division par zéro : `EvalError::DivisionByZero` ; un nœud `SetVar` la traite en no-op avec un avertissement (`Vm::take_warnings`), une `VarOp` en erreur sans effet. Une variable garde son type déclaré (`orbit_core::coerce` : un résultat décimal rangé dans un entier est arrondi).
* **VM** : `Vm::apply_var_op` et `Vm::eval_condition`. Une variable de partie modifiée hors d'un nœud est reportée dans le point de retour de l'écran affiché et coupe le futur connu (comme un autre choix) : revenir à cette réplique garde le changement, revenir avant l'annule ; les sauvegardes l'embarquent (elles contiennent `vars`). Une variable persistante est écrite dans le magasin persistant, jamais annulée.
* **UI** : `UiAction::VarSet(VarOp)` (remplace le stub), `Cond::Vars(Condition)` pour `visible` / `enabled`, `BindPath::Var` pour les labels (affichage), curseurs (nombre) et interrupteurs (booléen). `DataContext::vars` (variables des deux portées) ; `UiEvent::VarOp` appliqué par le shell (au titre, seules les variables persistantes changent). `DataScreen` applique l'opération localement pour réagir tout de suite, le shell renvoie les vraies valeurs. L'aperçu de l'éditeur tient des variables fictives (`PreviewNav::vars`, valeurs initiales déclarées) et montre « Variable: gold -> 15 ». `validate_with_vars` / `UiBundle::variable_issues` : variable inconnue, opération ou valeur d'un autre type (avertissements, branchés au panneau de diagnostic de l'éditeur). `SceneGoto` reste non pris en charge.
* **Éditeur** : `orbit_editor_api::varwidgets` (modèle pur testé + widgets) : menu de variables (déclarées dans l'histoire), menu d'opérations filtré par type, valeur typée (nombre / case / texte) ou autre variable, constructeur de conditions (lignes, ajout / retrait, groupes « toutes / au moins une »), bascule « Mode expression ». Utilisé par l'inspecteur (`SetVar`, `If`, conditions de choix) et par l'éditeur d'interface (action « Modifier une variable », condition « une condition sur une variable », liaisons).

## Addendum : sauvegardes, aperçu interactif

* **`SaveSlots { mode, columns, rows }`** (widget composite) : grille paginée de cartes (miniature, date, dernière réplique), onglets de catégorie (mode `Load`), pages, suppression ; sauvegarde dans un slot utilisé et suppression passent par une confirmation. Il réutilise les `UiEvent::SaveSlot / LoadSlot / DeleteSlot` et la logique de pagination de l'écran Rust ; les données viennent de `DataContext::saves` (rempli par le shell seulement si un écran en contient un). `slots_page` est pure et testée.
* `SYSTEM_SCREENS` gagne `save` et `load` : le shell les substitue aux écrans Rust (repli inchangé), y compris pour `OpenSave` / `OpenLoad`. `assets/ui/default/save.screen.ron` et `load.screen.ron` font partie de `UiBundle::builtin()` ; gabarits « Save » / « Load » dans l'éditeur.
* **Aperçu de l'éditeur** : `PreviewNav` (orbit_ui, sans GPU) tient la pile d'écrans de l'aperçu, les réglages et sauvegardes fictifs, et traduit les actions « de jeu » (GameNew, Quit, SceneGoto...) en toasts « Action: ... ». Open / Replace / Close / CloseAll / Back naviguent ; l'éditeur sélectionne l'écran affiché (`PreviewState::ui_shown`).
* **Sélection par clic** : `DataScreen::widget_at` renvoie le chemin (indices d'enfants) du widget le plus profond sous le pointeur, labels et panneaux compris (`PreviewState::ui_pick`) ; le widget sélectionné est entouré dans l'aperçu (`ui_highlight`).
* **Corrections de chaîne** : le dossier `ui/` du projet n'était jamais donné au runtime (ni par `orbit play <projet>`, ni par le Play de l'éditeur, qui forçait aussi les écrans Rust) : `native_config` et `PreviewHost::load` fixent maintenant `ui_dir`. Chaque action de bouton est testée au niveau du shell.

## Addendum : extraction de `orbit_ui_data` (UI-01)

* **Nouvelle crate `orbit_ui_data`** (dépend de `orbit_core`, `serde`, `ron` ; ni wgpu, ni police ; compilable en wasm sauf `UiBundle::load_dir`, réservé au natif). Elle contient `ScreenAsset`, `WidgetDef`, `WidgetKind`, `Cond`, `TextRef`, `LayoutDef`, `SlotsMode` (+ `from_ron` / `to_ron`, `UI_VERSION`), `UiAction`, `BindPath`, `SettingKey`, la migration `ui_version` (`migrate`), la validation (`validate`, `Issue`, `Severity`), `UiBundle` (+ `SplashDef`, `SYSTEM_SCREENS`, gabarits intégrés `assets/ui/default/`), le `Theme` sérialisable avec ses styles (`Style`, `Fill`, `Border`, `StateColors`, `resolve_style`...), le vocabulaire de mise en page sérialisé (`Size`, `Align`, `Dir`, `Anchor`, `PanelKind`) et `UiSound`.
* **`orbit_ui` réexporte** tout sous les chemins historiques (`orbit_ui::asset::*`, `orbit_ui::{Theme, Anchor, UiBundle...}`) : aucun appelant n'a changé. Restent dans `orbit_ui` ce qui a besoin des widgets ou du rendu : `DataScreen`, `PreviewNav`, `slots_page`, `tint` (conversion sRGB via `orbit_text`).
* **`orbit_export` dépend d'`orbit_ui_data` seulement** : `cargo tree -p orbit_export` ne contient plus wgpu ni les crates de rendu (le compromis du premier addendum est levé). `orbit_pak` et `orbit_mcp` n'en dépendaient déjà pas.
* **Formats inchangés, octet pour octet** : aucun champ, attribut serde ni valeur par défaut n'a bougé, `UI_VERSION` reste identique. Garde-fou : `crates/orbit_ui_data/tests/format_stability.rs` compare la sérialisation du bundle intégré, du thème par défaut et des écrans stock à des fichiers « golden » produits par l'ancien code (`tests/golden/`), et vérifie leur relecture. Les tests qui ont besoin de `DataScreen` (écrans stock constructibles) sont restés dans `orbit_ui`.

## Addendum : `UiAction::SceneGoto` (UI-09)

* **Aucun changement de format** : `SceneGoto(String)` existait déjà dans `UiAction` (réservé) ; ni schéma d'histoire, ni `ui_version`, ni `GamePack`, ni format de sauvegarde ne bougent. Les écrans et sauvegardes existants se chargent tels quels. La chaîne désigne la scène par son **id** (texte ULID, ce qu'écrit l'éditeur : stable au renommage) ; un **nom** de scène est aussi accepté (écrit à la main), l'id l'emporte.
* **VM** : `Vm::find_scene(&str)` (id puis nom) et `Vm::goto_scene(SceneId)`, saut hors nœud : `pc` = premier nœud de la scène, pile d'appels vidée, attente en cours (choix, pause, transition) abandonnée, erreur précédente effacée. Variables, RNG et scène déclarative (fond, personnages, musique) sont **gardés** : la scène cible pose son propre décor. Scène inconnue : `VmError::UnknownScene`, rien ne change ; scène vide : fin de l'histoire.
* **Rollback / historique** : le saut se comporte comme un autre choix. Le point de retour de l'écran affiché reste le point de rollback (`History::on_goto` : le futur connu est coupé, `ahead` est levé), donc un pas de rollback ré-affiche la réplique vue au moment du saut avec l'état d'avant le saut (la suite reprend la scène d'origine) ; le backlog continue naturellement avec les répliques de la scène cible. Aucun point de retour supplémentaire n'est créé (le prochain `Say`/`Choice` en crée un comme d'habitude).
* **Sauvegardes** : rien de spécial, l'état après le saut est un `VmState` ordinaire et l'historique sérialisé traverse le saut (testé : sauvegarde/chargement puis rollback avant le saut).
* **Shell** : `UiEvent::SceneGoto` ; en partie, `Game::goto_scene` (coupe saut rapide et auto, comme un choix) → `Director::goto_scene` (abandonne pause/transition, exécute jusqu'à la prochaine réplique), puis tous les écrans sont fermés. Au titre (pas de partie) : ignoré avec un journal. `UiAction::is_supported` vaut désormais toujours `true`.
* **Aperçu de l'éditeur** : pas d'histoire en cours, l'action est affichée en toast « Action: SceneGoto(...) ».
* **Éditeur / validation** : l'action « Aller à une scène » propose la liste des scènes de l'histoire (affiche le nom, stocke l'id ; champ texte libre sans histoire). `validate` : cible vide = erreur ; `scene_issues` / `UiBundle::scene_issues` : scène inconnue = avertissement (panneau de diagnostic de l'éditeur d'interface).
