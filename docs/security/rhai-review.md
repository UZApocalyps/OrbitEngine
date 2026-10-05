# Revue de sécurité : moteur de scripts Rhai

* **Date** : 2026-10-03
* **Portée** : `crates/orbit_script` (configuration de l'`Engine`, API, résolveur de modules, conversions de valeurs), et ses appelants : `orbit_vm` (`ScriptHost`, conditions Rhai, application transactionnelle), `orbit_runtime` (chargement des modules depuis le pak ou le dossier), `orbit_export` (modules embarqués), `orbit_project` (lecture / écriture des modules, `test_run`), `orbit_mcp` (`test_script`, `check_scripts`), l'éditeur (« Exécuter dans le bac à sable ») et la CLI (`orbit validate`, `orbit run-text`). Survol rapide de `orbit_plugin_api` et du plugin d'exemple `dice`.
* **Version** : Rhai 1.26.1, features `std`, `sync`, `no_time`, `no_custom_syntax` (+ `no_closure` après revue), sans `unchecked`.
* **Exigence** : ADR 0015 (revue opus de la configuration de l'Engine avant la sortie).

## Modèle de menace

Un jeu téléchargé est **non fiable** : son pak (histoire, scripts des nœuds, conditions Rhai, modules `scripts/<nom>.rhai`) est écrit par un tiers. Un projet ouvert dans l'éditeur, ou reçu par le serveur MCP, l'est aussi. Ce qu'un script peut faire de pire doit se terminer en **erreur de script** :

1. aucune évasion du bac à sable (fichiers, réseau, horloge, code chargé depuis une chaîne, modules hors de la mémoire) ;
2. aucun crash ni gel du lecteur ou de l'éditeur (déni de service : un débordement de pile ou une allocation hors limite ferme le processus, et dans l'éditeur fait perdre le travail non enregistré) ;
3. aucun changement d'état en cas d'échec (variables, RNG) ;
4. le même résultat sur toutes les plateformes (sauvegardes, retours en arrière, rejeux).

Un crash du lecteur provoqué par le jeu lui-même n'expose pas de données, mais il reste inacceptable : il peut aussi venir d'une **erreur de bonne foi** (un module qui s'importe lui-même) et il frappe l'éditeur, qui ouvre des projets de tiers.

## Méthode

Lecture du code (`orbit_script`, appelants, sources de Rhai 1.26.1 : `eval/chaining.rs`, `eval/data_check.rs`, `packages/*`), puis sondes adverses exécutées sur un fil à **pile de 1 Mio** (fil principal d'un lecteur Windows, pile par défaut de wasm), en debug et en release. Les sondes sont devenues la suite `crates/orbit_script/tests/adversarial.rs` (22 tests, dont un fuzzing par soupe de jetons sur 3 000 programmes : ni crash, ni boucle, ni effet de bord en cas d'échec).

## Constats

Gravité : **Critique** (crash ou gel atteignable par un script trivial, y compris par erreur), **Élevée** (crash / épuisement atteignable par un script hostile), **Moyenne**, **Faible**, **Info**.

### R1 — Import cyclique : débordement de pile, crash (Critique, corrigé)

`a` importe `b` qui importe `a` (ou un module qui s'importe lui-même) faisait déborder la pile native : crash immédiat du lecteur et de l'éditeur. Cause : `Module::eval_ast_as_new` évalue chaque module avec un **état d'exécution neuf** ; les compteurs `max_modules` et de profondeur d'appel de Rhai repartent de zéro à chaque niveau, rien n'arrête la récursion.

**Correctif** : le résolveur en mémoire tient, dans l'état de l'appel, la pile des modules en cours d'import : un cycle est une erreur (`cyclic import: a -> b -> a`), la profondeur est bornée à `MAX_IMPORT_DEPTH` = 8, et le nombre total de modules évalués par appel à `max_modules` (16), imports en boucle et imports des modules compris.

Tests : `cyclic_imports_fail_instead_of_overflowing_the_stack`, `imports_nest_a_bounded_depth`, `modules_count_against_the_call`, `deepest_legal_recursion_fits_a_small_stack` (8 niveaux d'import × 32 niveaux d'appel × expressions profondes tiennent dans 1 Mio).

### R2 — `sleep` disponible : gel du jeu (Critique, corrigé)

Le paquet standard de Rhai (`LangCorePackage`, hors `no_std`) expose `sleep(secondes)`. `sleep(1e9)` gelait le jeu indéfiniment ; sur wasm, `std::thread::sleep` panique. `disable_symbol("sleep")` est sans effet (ce n'est pas un mot-clé).

**Correctif** : `sleep(INT)` et `sleep(FLOAT)` sont surchargés sur l'Engine (les fonctions enregistrées passent avant les paquets) et renvoient une erreur. Test : `no_host_access`.

### R3 — Valeurs imbriquées : débordement de pile (Élevée, corrigé)

`let a = []; loop { a = [a]; }` construit une valeur aussi profonde que les limites de taille le permettent (Rhai additionne les éléments des tableaux imbriqués : profondeur ≤ `max_array_size` + `max_map_size`, soit 20 000 avec les anciennes valeurs). Mesures sur 1 Mio :

| Opération | debug | release |
|---|---|---|
| construction, copie, destruction, `to_json` | OK à 2 000 | OK à 2 000 |
| `to_string`, `==`, `!=`, `print`, `debug`, interpolation | crash entre 500 et 700 | **crash dès 500** |
| construction à 9 000 (pile de test de 2 Mio) | crash | — |

Les fonctions d'affichage et d'égalité de Rhai se rappellent elles-mêmes par le répartiteur de fonctions (grosses trames) pour chaque niveau.

**Correctifs** :

* `max_array_size` / `max_map_size` ramenés de 10 000 à **1 024** : profondeur constructible ≤ 2 048, zone mesurée sans danger pour la construction, la copie et la destruction ;
* `to_string`, `to_debug`, `print`, `debug`, `==`, `!=` sont surchargés pour les tableaux et les tables : même format et même sémantique que Rhai, mais la profondeur est contrôlée d'abord (`MAX_VALUE_DEPTH` = 64, parcours qui s'arrête à la limite). Un affichage trop profond ne lève pas d'erreur sur place (l'interpolation `${…}` et la concaténation avalent l'erreur et retombent sur un `Display` récursif) : il rend un texte de remplacement et marque l'appel, que l'opération suivante (ou la fin du script) transforme en échec ;
* l'audit de portée (R5) refuse aussi toute variable plus profonde que 64.

Tests : `nested_values_are_bounded_before_the_stack_is` (tableaux, tables, alternés, `[a, a]` ; `to_string`, interpolation, `+`, `==`, `!=`, `contains`, `in`, `switch`, `print`, `debug`, copie), y compris en `--release`.

### R4 — Rappels et modules hors budget d'opérations (Élevée, corrigé)

Rhai compte les opérations par état d'exécution ; les rappels passés à `sort`, `map`, `filter`, `reduce`, `retain`, `all`, `dedup`, `index_of`… et les modules reçoivent le leur. Un comparateur de `sort` pouvait donc dépenser 100 000 opérations **par comparaison** (10⁴ éléments ⇒ ~10¹⁰ opérations : gel de plusieurs minutes). De plus `sort` avale les erreurs du comparateur, y compris le dépassement.

**Correctif** : compteur atomique par appel incrémenté par `on_progress` (appelé à chaque opération, quel que soit l'état) ; au-delà de `max_operations` chaque opération échoue, le script entier aussi. Test : `callbacks_share_the_operation_budget` (huit formes de rappel, plus une boucle sur `range(0, i64::MAX)` et `loop {}`), `recursion_through_callbacks_is_bounded`.

### R5 — `map[clé] = valeur` non contrôlé : épuisement mémoire (Élevée, corrigé)

Dans `eval/chaining.rs`, l'affectation indexée ne vérifie que la **valeur affectée**, pas la table : `for i in 0..15000 { m[`${i}`] = s + i; }` (`s` de 60 Ko) dépassait le gigaoctet en 200 ms. De même, des centaines de variables, chacune sous sa limite, n'ont aucun plafond commun.

**Correctif** : `on_var` (appelé à chaque lecture de variable ; la variable de la table est lue avant chaque affectation) mesure toute la portée courante (et `this`) toutes les 32 lectures : limites de Rhai par valeur (éléments ≤ `max_array_size` + `max_map_size`, texte ≤ `max_string_size`), profondeur ≤ 64, et budgets totaux `max_scope_elements` (32 768) et `max_scope_string_bytes` (4 Mio). Dépassement ⇒ erreur « … is too large (size limit) ». Le coût est borné (portée bornée par ces budgets, une mesure toutes les 32 lectures).

Tests : `growing_a_map_by_index_is_bounded` (table plate, imbriquée, dans une fonction, 200 variables), `sizes_are_limited`.

### R6 — Fermetures capturantes et `curry` (Moyenne, corrigé)

Les valeurs capturées par une fermeture ou par `curry` vivent dans des pointeurs de fonction que les contrôles de taille de Rhai ne voient pas : `f = Fn("x").curry(f)` en boucle, ou `let g = f; f = || g.call()`, imbriquaient sans borne (débordement de pile observé à 9 000 niveaux). Les fermetures créent aussi des valeurs partagées capables de former des cycles (fuite mémoire).

**Correctif** : feature `no_closure` (les fonctions anonymes restent permises mais ne capturent rien ; aucun script du dépôt n'en capturait) et `disable_symbol("curry")`. Test : `no_curry_and_no_capturing_closures`.

### R7 — Flottants non finis dans l'histoire (Faible, corrigé)

`set_var("ratio", 0.0 / 0.0)` stockait `NaN` : comparaisons toujours fausses, propagation dans tous les calculs, valeur « changée » à chaque comparaison de la VM. **Correctif** : `NaN` et ±∞ sont refusés à la conversion vers une valeur d'histoire (variables, listes, tables, résultat d'une condition). Test : `non_finite_floats_never_reach_the_story`.

### R8 — Panique dans Rhai (Faible, défense en profondeur)

Aucune panique trouvée (arithmétique vérifiée, indices hors bornes, tranches, `get_bits`, `parse_int` à base invalide, `rand_int(i64::MIN, i64::MAX)`… sont des erreurs ou sont bornés ; voir `arithmetic_edges_are_errors_not_panics`, `the_api_survives_extreme_arguments`). Par précaution, l'exécution est enveloppée dans `catch_unwind` : une panique de Rhai devient un échec du script là où le déroulement existe (pas sur wasm, compilé en `panic = abort`). Le `expect` sur l'état de l'appel est remplacé par un repli.

### R9 — Collision de clé du cache d'AST (Faible, corrigé)

Un script dont le texte commençait par `\0expr\0` partageait la clé du cache avec la condition de même suite et exécutait l'AST de l'autre sorte. **Correctif** : les deux sortes sont étiquetées (`s\0…`, `e\0…`).

### R10 — Plugin `dice` : débordement d'entier (Faible, corrigé)

`modifier` vient des données du nœud (fichier du jeu) : `somme + i64::MAX` paniquait en debug et repliait en release. **Correctif** : `checked_add`, échec propre du nœud (rien n'est appliqué). Test : `hostile_node_data_fails_cleanly`. Le reste de `orbit_plugin_api` (registre, `PluginContext::random_range` sans biais, `apply` transactionnel) n'appelle pas de remarque ; les plugins sont du Rust lié statiquement (ADR 0005), de confiance par construction.

### Vérifiés sans changement (Info)

* **`eval`** : `disable_symbol("eval")` ; `Fn("eval")` et `call(Fn("eval"), …)` sont refusés à la compilation (mot réservé). Test : `no_code_from_strings`.
* **Entrées / sorties, horloge** : aucune fonction de fichier, réseau ou processus dans le paquet standard ; `timestamp()` absent (`no_time`). `print` / `debug` / `log` vont dans les journaux du script, plafonnés à 64 entrées de 1 024 caractères.
* **Imports hors de `scripts/`** : le résolveur ne consulte qu'une table en mémoire ; `../shop`, `/etc/passwd`, `C:\…`, `\\serveur\partage`, `file:///…`, `scripts/shop.rhai` sont « introuvables ». Le chargeur du lecteur (`orbit_runtime::scripts`) ne demande au pak que `scripts/<nom>.rhai` pour des noms valides (`[A-Za-z0-9_-]{1,64}`) ; le projet (`list_modules`) ignore les liens symboliques et les noms invalides ; l'export n'embarque que ces modules. Test : `imports_only_reach_in_memory_modules`.
* **Syntaxe personnalisée** : `no_custom_syntax`. **Fonctions hôtes** : seules les fonctions de l'API (`get_var`, `has_var`, `set_var`, `rand_int`, `rand_float`, `chance`, `branch`, `log`, `api_version`) et les surcharges ci-dessus ; les conditions sont en lecture seule.
* **Complexité du source** : parenthèses, blocs, `!`, `-`, accès `.a` / `[0]` imbriqués 10⁵ fois, sommes de 2·10⁵ termes : erreur « Expression exceeds maximum complexity », sans récursion profonde. Test : `source_complexity_is_limited`.
* **Arithmétique** : pas de feature `unchecked` ; débordements, division par zéro, `i64::MIN / -1`, `abs`, puissance : erreurs. `range` à pas nul : erreur ; plages immenses : bornées par les opérations.
* **Transactions** : un échec (erreur, limite, module en échec, valeur refusée) n'applique rien : `PluginContext` ne garde les écritures et les tirages RNG que si le script réussit (`commit_plugin`). Tests : `a_failing_module_applies_nothing`, `wrong_values_cannot_be_stored`, et l'assertion « aucun changement » de chaque test en échec.
* **Déterminisme** : tables en `BTreeMap` (ordre des clés stable), formatage des flottants par Rust (indépendant de la plateforme), RNG de la VM seul (pas de source d'aléa ni d'horloge). Test : `same_state_same_result`.

## Risques résiduels

1. **Flottants transcendants** : `sqrt` et les quatre opérations sont exacts (IEEE 754), mais `sin`, `cos`, `exp`, `ln` et `**` flottant passent par la libm de la plateforme ; natif et wasm peuvent différer d'un ulp. Un jeu qui compare ces résultats à l'égalité stricte pourrait diverger entre plateformes (pas entre deux exécutions sur la même). À documenter pour les auteurs ; une libm Rust pure (`libm`) unifierait le comportement si cela devient un besoin.
2. **Mémoire des modules** : un module importé garde ses variables de module hors de la portée auditée ; ≤ 16 modules par appel, chacun audité pendant son évaluation : plafond de l'ordre de 16 × 4 Mio de textes.
3. **Fenêtre de l'audit** : entre deux mesures (32 lectures), une table peut grandir de 32 affectations (≈ 4 Mio au pire avec des textes de 64 Ko) et une valeur gagner quelques dizaines de niveaux par expression littérale ; les marges mesurées (construction sûre jusqu'à 2 000 niveaux, affichage contrôlé à 64) couvrent cette fenêtre.
4. **Dépendance à Rhai** : plusieurs protections reposent sur des détails de Rhai 1.26 (priorité des fonctions enregistrées sur les paquets, `on_var` marqué « volatile », état neuf des modules et des rappels). `Cargo.lock` fige la version ; **relancer `cargo test -p orbit_script --test adversarial` (debug et `--release`) à chaque mise à jour de Rhai**.
5. **Pile de wasm** : les mesures sont faites sur 1 Mio natif ; la pile de wasm combine la pile de mémoire linéaire (1 Mio par défaut) et celle du navigateur. Les limites (8 imports, 32 appels, 64 niveaux de valeur) laissent une marge large, mais aucun test automatique ne tourne dans un navigateur.
6. **Temps processeur** : un script peut toujours dépenser son budget (100 000 opérations, quelques dizaines de millisecondes) à chaque exécution d'un nœud ; une histoire hostile qui boucle sur un tel nœud ralentit le jeu sans le bloquer, comme toute histoire mal écrite.
7. **`catch_unwind`** est sans effet sur wasm (`panic = abort`) : une panique de Rhai y resterait fatale (aucune connue).

## Résumé des changements

* `crates/orbit_script/Cargo.toml` : feature `no_closure`.
* `crates/orbit_script/src/lib.rs` : résolveur avec détection de cycle, profondeur et compte d'imports ; compteur d'opérations global (`on_progress`) ; surcharges `sleep`, affichage et égalité des tableaux / tables avec contrôle de profondeur ; audit de portée (`on_var`) ; `curry` désactivé ; tailles 1 024 ; nouveaux champs `Limits::max_scope_elements` / `max_scope_string_bytes` ; constantes publiques `MAX_IMPORT_DEPTH`, `MAX_VALUE_DEPTH` ; refus des flottants non finis ; `catch_unwind` ; clé de cache étiquetée ; messages d'erreur des limites lus au travers des erreurs de module.
* `crates/orbit_script/tests/adversarial.rs` : suite adverse (pile de 1 Mio, fuzzing).
* `examples/plugins/dice/src/lib.rs` : `checked_add` et test.
* Manuel (en / fr, chapitre 14) et ADR 0015 (limites, addendum).
