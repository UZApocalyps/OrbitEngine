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
