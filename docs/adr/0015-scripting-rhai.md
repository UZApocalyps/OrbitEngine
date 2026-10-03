# ADR 0015 : Scripting Rhai, sandbox, plugins Rust et DSL texte

Statut : proposé (M8-00 et M8-06, à relire avant M8-01). Complète ADR 0005.

## Contexte

ADR 0005 a reporté le scripting au M8 et posé des invariants : l'état de jeu ne change que via l'API du VM (store snapshotté), RNG seedé et sauvegardé, effets externes réversibles ou marqués non annulables. Le public reste no-code : le script est un **complément optionnel**, jamais un prérequis. Contraintes : natif **et** web (wasm32), rollback/saves exacts, aucun accès système depuis un jeu distribué, compatibilité ascendante (§1.3).

## Options examinées

### Langage

1. **Rhai** (retenu, confirmé). Rust pur, compile en wasm32 sans JS, moteur embarquable avec limites natives (opérations, profondeur, taille des chaînes/tableaux), pas d'I/O par défaut, syntaxe proche de JS/Rust lisible par des débutants. Inconvénient : performances modestes (interpréteur AST) — sans importance pour de la logique de VN.
2. **Lua (mlua).** Écarté : dépend d'une lib C (cross-compilation wasm32-unknown-unknown pénible), sandbox à construire à la main.
3. **WASM invité (wasmtime/wasmi).** Écarté pour les auteurs : chaîne de compilation hors éditeur, contraire au no-code ; garde une place en backlog pour les plugins desktop (ADR 0005).
4. **Étendre le langage d'expressions existant.** Insuffisant pour des boucles ou des fonctions, mais **reste** le langage des conditions et `SetVar` : Rhai ne le remplace pas.

### Où le script s'exécute

1. **Nœud `Script` + expressions Rhai optionnelles, exécution synchrone et bornée au pas du VM** (retenu). Le script ne fait que lire des variables et produire des opérations ; le VM les applique. Le rollback reste celui du VM.
2. **Coroutines / scripts longs qui `await` le joueur.** Écarté : l'état d'une coroutine Rhai n'est pas sérialisable → saves et rollback impossibles. Le flux narratif reste dans le graphe ; un script qui veut attendre retourne une valeur qui oriente une branche.

## Décisions

### 1. Crate et intégration

* `orbit_script` (feature `script`, compilable wasm) : enveloppe `rhai::Engine` configuré une fois (`Engine::new_raw` + paquets choisis), cache d'AST compilés par hash de source.
* Le VM appelle `orbit_script` via un trait `ScriptHost` ; sans la feature, un nœud `Script` produit une erreur de validation claire (« ce projet utilise des scripts ») au lieu d'un crash.
* IR : `NodeKind::Script { source: String, outputs: Vec<PortId> }` et `Expr::Rhai(String)` en option dans les conditions (choix, `If`). Le script est **inline dans l'IR** (diffable, migrable) ; les modules partagés sont des fichiers `scripts/*.rhai` déclarés dans le manifeste, importables par nom (`import "inventaire"`), résolus par un `ModuleResolver` en mémoire (aucun accès disque au runtime ; le pak embarque les sources).

### 2. API exposée (surface minimale, versionnée)

* `var(name)` / `set_var(name, value)` : lecture directe ; l'écriture **ne modifie pas** le store, elle empile une `VarOp` appliquée par le VM après le script, avec contrôle de type (coercion ADR 0011 addendum v7). Un script qui échoue n'applique rien (transactionnel).
* Types : entiers, décimaux, booléens, textes, **listes et maps** (ADR 0016, conversion `Value` ↔ `Dynamic`).
* `rand_int(a, b)`, `rand_float()`, `chance(p)` : **RNG du VM** (seedé, sauvegardé) ; le `rand` de Rhai n'est pas enregistré.
* `goto(port)` / valeur de retour = port de sortie choisi ; `log(msg)` vers la console de l'éditeur et les avertissements du VM ; `t(key)` pour un texte localisé.
* Pas d'horloge réelle, pas d'I/O, pas de `eval`, pas d'accès aux autres scripts hors `import`. `API_VERSION` exposé ; tout ajout est additif.

### 3. Limites (sandbox)

* `set_max_operations` (100 000 par appel par défaut), `set_max_call_levels` (32), `set_max_expr_depths`, `set_max_string_size` (64 Ko), `set_max_array_size` / `set_max_map_size` (10 000), `set_max_modules`. Dépassement ⇒ erreur de script : le nœud prend sa sortie d'erreur s'il en a une, sinon il est ignoré avec avertissement, et l'éditeur l'affiche dans Problèmes avec la ligne.
* Déterminisme : mêmes entrées (variables + état RNG) ⇒ mêmes sorties, natif et web (pas d'itération sur `HashMap` non ordonnée exposée : maps en `BTreeMap`). Test de propriété : rejouer depuis un snapshot donne le même état.
* Mémoire : bornée indirectement par les tailles ci-dessus ; pas d'allocateur dédié (coût/bénéfice faible).

### 4. Plugins Rust (M8-04)

* `orbit_plugin_api` (`API_VERSION` = 1) : traits `NodeHandler` (pour `NodeKind::Custom { plugin, kind, params: Value }`) et `UiActionHandler` (pour `UiAction::Custom`). Mêmes règles que les scripts : ils reçoivent une vue en lecture et renvoient des opérations.
* Liaison **statique** (pas de dylib, ADR 0005) : l'export « avec plugins » génère un petit crate player qui dépend des crates plugins et lance `cargo build` ; nécessite une toolchain Rust (détectée et expliquée par le panneau Export). Sans plugin, l'export utilise le player précompilé comme aujourd'hui.
* Un jeu qui référence un plugin absent : validation en erreur à l'export, avertissement + nœud ignoré au Play.

### 5. DSL texte optionnel (M8-06/07)

* `orbit_dsl` : **vue** texte de l'IR (style proche Ren'Py/Ink : `label`, répliques `alice "Bonjour"`, `menu`, `if`, `$ gold += 1`, blocs `script { … }`). Ce n'est **pas** un nouveau format source : l'IR RON reste la vérité.
* IDs ULID conservés par des annotations discrètes en fin de ligne (`#@01J…`) masquables dans l'éditeur ; une ligne sans ID est un nouveau nœud. Les données sans syntaxe DSL (positions dans le graphe, mise en scène fine) sont conservées par fusion : le parser produit un diff appliqué à l'IR existante via les commandes `orbit_project`, pas un remplacement.
* Round-trip testé par propriétés (`proptest`) : IR → texte → IR identique ; texte → IR → texte stable (formatage canonique).
* Éditeur : vue texte synchronisée par scène, erreurs de syntaxe dans Problèmes, application sur validation (pas à chaque frappe).
* *Alternative écartée* : DSL comme format de stockage — deux vérités, migration double.

### 6. Schéma et compatibilité

* Les ajouts (`NodeKind::Script`, `Expr::Rhai`, `NodeKind::Custom` si absent, section `scripts` du manifeste) sont **additifs** et prennent le prochain numéro de schéma libre au moment de M8-01 (après v12/M6 et M7-05). Migration = changement de numéro.
* Un projet antérieur ne contient aucun script : comportement inchangé, fixture compat `vN` figée et test standard. Le pak gagne une section optionnelle de sources ; décodage des versions précédentes conservé.
* Saves : aucun état de script n'est sauvegardé (pas de coroutine) ; une save antérieure se charge telle quelle. Un runtime sans feature `script` ouvrant un jeu scripté refuse proprement avec message.

## Conséquences

* Le plan M8 est conservé ; précisions : M8-01 inclut l'API transactionnelle et le `ModuleResolver` en mémoire ; M8-06 inclut les annotations d'ID et la fusion par diff.
* Le scripting ne peut pas exprimer d'attente (choix assumé) ; les mini-jeux temps réel passent par un plugin Rust ou un écran UI (ADR 0016).
* Revue sécurité (opus) obligatoire sur la configuration de l'`Engine` avant fusion de M8-01.

## Addendum M8-04 : plugins Rust runtime (implémenté)

Approches examinées pour exécuter `NodeKind::Custom` :

1. **Intercepter `PresentationEvent::Custom` dans le player, après coup.** Écarté : l'événement arrive quand le VM est déjà passé au nœud suivant, donc un plugin ne peut ni bloquer, ni influencer un `If` qui suit dans le même `step()`.
2. **Trait hôte dans `orbit_vm`, implémenté par `orbit_plugin_api`** (retenu). `orbit_vm::PluginHost` (`run_node`, `run_action`) est attaché à chaque `Vm` (`set_plugin_host`), hors `VmState` : aucun format de save, de pak ni d'histoire ne change. `orbit_plugin_api::RuntimeRegistry` l'implémente ; le runtime ne dépend que d'`orbit_vm`.
3. **Plugins dynamiques (dylib/WASM).** Écarté (ADR 0005) : liaison statique.

Décisions :

* `API_VERSION` = 1. `RuntimeRegistry::install` et `ProjectRegistry::install` refusent toute autre version (`PluginError::IncompatibleApi`), un id invalide ou un doublon.
* Handlers `NodeHandler` / `UiActionHandler` : `&self`, `Send + Sync`. Ils reçoivent un `PluginContext` : lecture des variables (avec leurs propres écritures en attente), `eval(Condition)`, RNG seedé du VM (`random_u64`, `random_range` sans biais, `random_f64`), `apply(VarOp)` / `set`, `emit(événement)`, `warn`, `fail`. Le VM applique tout **après** le handler, en tout ou rien : une opération refusée ou un `fail` annule le nœud (RNG non avancé) avec un avertissement.
* Résultat d'un nœud : `PluginOutcome::Continue` ou `Wait(secondes)` (→ `StepResult::WaitTime`). Pas d'attente d'entrée joueur ni de saut de port (les nœuds custom n'ont qu'une sortie) : un mini-jeu interactif passe par un écran UI dont les boutons appellent des actions custom.
* Événements : `PresentationEvent::Custom { plugin_id, kind: nom, data }` ; l'événement générique du nœud reste émis en premier (compatibilité éditeur).
* **Rollback et saves** : un plugin ne garde aucun état de jeu. Son état « sérialisable déclaré » = des variables de l'histoire, désignées par des champs `FieldKind::Variable(type)` que la validation vérifie (variable déclarée, bon type). Le RNG est dans `VmState`. Rejouer depuis un snapshot redonne le même résultat (testé).
* `UiAction::Custom { name: "<plugin>.<action>", args }` (UI-18) : la `Shell` l'envoie au VM (`Vm::run_ui_action`), dont les écritures patchent le checkpoint affiché comme `apply_var_op` ; au titre, seules les variables persistantes existent. Sans handler : `Effect::Custom` vers l'hôte, comme avant.
* Plugin absent au Play : nœud ignoré + avertissement. À l'export : erreur si un plugin requis n'est fourni par aucune crate de `plugins/`.
* Schéma de champs : `NodeTypeDef::fields: Vec<FieldDef>` (`Int`/`Float` bornés, `Bool`, `Text`, `Variable`, `Choice`) → inspecteur typé dans l'éditeur et validation (`check_fields`).
* Export « avec plugins » (`orbit_export::plugins`) : crates dans `plugins/<crate>/`, ids déclarés dans `[package.metadata.orbit] plugins = [...]`, fonction `orbit_plugins()`. Le panneau Export détecte `cargo` ; l'exporteur génère `Cargo.toml` + `src/main.rs` + `src/plugins.rs` (crate hors workspace) qui appelle `orbit_player::run_with_plugins`, lance `cargo build --release` et utilise l'exécutable comme template. Sources du moteur : `ORBIT_ENGINE_SRC` ou l'arbre de compilation. Seule la plateforme hôte (Windows/Linux) est supportée ; web/Android avec plugins : refus explicite (à faire).
* Hooks de dessin : non fournis (reportés) ; un plugin présente ses résultats via des événements ou des variables affichées par l'UI.

## Addendum M8-01 / M8-02 / M8-03 / M8-05 : scripting implémenté

* **Crate `orbit_script`** (Rhai 1.26, `default-features = false`, features `std`, `sync`, `no_time`, `no_custom_syntax`) : `Engine::new_raw` + `StandardPackage`, `eval` désactivé, résolveur de modules **en mémoire** (aucun accès disque), `print`/`debug` capturés en logs, variables strictes. Limites (`Limits`) : 100 000 opérations, 32 niveaux d'appel, profondeur d'expression 64/32, chaînes 64 Ko, tableaux et maps 10 000, 16 modules. Compile en wasm32.
* **Hook VM** : `orbit_vm::ScriptHost` (`run`, `eval`), attaché comme `PluginHost` (`Vm::set_script_host`, hors `VmState`). Le script travaille sur un `PluginContext` : écritures et tirages RNG appliqués par le VM **après** le script, tout ou rien (même chemin que les plugins) ; un échec n'applique rien, RNG compris. Sans hôte : nœud ignoré avec avertissement, condition Rhai lue `false`.
* **Noms de l'API** : `var` et `goto` sont des **mots réservés de Rhai** ; l'API est donc `get_var`, `has_var`, `set_var`, `rand_int`, `rand_float`, `chance`, `branch("nom")` (ou valeur de retour texte), `log`, `api_version()` (`API_VERSION` = 1). `t(key)` n'est pas fourni (reporté).
* **IR v15** : `NodeKind::Script { source, branches: Vec<ScriptBranch { name, target }>, on_error: Option<NodeId> }` (cibles = `Label`) et `Expr::Rhai(String)`, autorisée seulement comme condition entière d'un `If` ou d'une option (sinon `ValidationError::InvalidScript`). Texte : `rhai: …` (`Expr::parse`), DSL `script "<source>" [branch n -> l]* [on_error l]` et `rhai "<code>"`. Une condition Rhai qui échoue vaut `false` avec avertissement. Migration v14→v15 = numéro ; fixture figée `examples/fixtures/compat/v14/`.
* **Modules** : fichiers `scripts/<nom>.rhai` du projet, **découverts** (pas de déclaration dans le manifeste, écart assumé : une source de vérité de moins). Réévalués à chaque `import` (aucun état entre deux scripts). Éditeur et MCP écrivent via `orbit_project::scripts::write_module` (écriture atomique, nom vérifié ; pas d'undo, comme les rigs). Le runtime charge les modules importés (transitivement) par le chargeur de l'histoire : pak ou dossier.
* **Pak** : les modules importés sont des **entrées** `scripts/<nom>.rhai` du pak ; `NodeKind::Script` / `Expr::Rhai` sont des variantes ajoutées en dernier (postcard additif). **Pas de changement de layout, pas de bump** de `GamePack` (reste 3/4/5). Un ancien lecteur refuse un jeu scripté par une erreur de décodage (« corrupt ») ; un lecteur récent sans scripting le refuse avec un message explicite.
* **Lecteurs** : feature `script` (`orbit_runtime`, `orbit_player`, défaut du lecteur desktop et de l'éditeur). Templates : natifs et Android avec `script` ; web : `web/` sans Rhai, **`webscript/`** (2D + scripts) et `web3d/` (3D + scripts) ; l'export choisit selon le jeu. Tailles mesurées (release-wasm + wasm-opt -Oz) : `web/` 5 444 332 o (1 845 529 brotli), `webscript/` 6 360 112 o (2 076 753 brotli), soit +916 Ko / +231 Ko brotli. Budget `webscript` : 6 500 000 / 2 150 000. Note : le `web/` 2D dépasse déjà son budget (4 700 000 / 1 630 000) indépendamment de ce travail (Rhai n'y est pas lié).
* **Licences** : Rhai MIT/Apache-2.0 ; dépendance `smartstring` en **MPL-2.0** (copyleft faible, fichier par fichier, non modifiée) : exception ajoutée dans `deny.toml`, comme `triple_buffer`.
* **Outils** : éditeur (inspecteur de script avec coloration, diagnostics ligne/colonne dans Problèmes, bouton « Exécuter dans le bac à sable », panneau Scripts pour les modules, case Rhai sur les conditions) ; MCP (`list_scripts`, `read_script`, `set_script`, `write_script_module`, `delete_script_module`, `check_scripts`, `test_script`, `add_command type=script`) ; `orbit validate` et `orbit run-text` (modules du dossier de l'histoire).
* **Reste à faire** : sorties de branche du nœud Script dans la vue graphe (le modèle `orbit_project::graph` ne connaît pas encore ses ports) ; `t(key)` ; revue sécurité opus de la configuration de l'Engine.
