# ADR 0016 : Écrans de gameplay, HUD et valeurs liste/map

Statut : UI-13 implémenté au **schéma v11** (voir l'addendum « Implémentation UI-13 » en fin de document) ; UI-14, UI-15 et UI-16 implémentés au **schéma v12** (saves format 5, `ui_version` 5 ; addendum en fin de document). Complète ADR 0011 et ses addenda.

## Contexte

Les écrans data-driven (ADR 0011) ne s'ouvrent que depuis d'autres écrans ; l'histoire ne peut pas afficher une carte, un téléphone ou un inventaire, ni un HUD permanent (jauge, or). Les variables n'ont que des scalaires (entier, décimal, booléen, texte) : un inventaire ou une liste de contacts demande des collections. Les deux besoins devaient arriver ensemble au schéma v10 (plan §9) ; v10 a été pris par M5a / M6-07 (effets, timelines, scène 3D) : UI-13 est livré au **schéma v11**, UI-14 prendra le numéro libre suivant. Contraintes : rollback, skip, saves exacts ; compatibilité ascendante (§1.3) ; tout faisable sans code (§1.4).

## Options examinées

### Collections

1. **`Value::List(Arc<Vec<Value>>)` et `Value::Map(Arc<BTreeMap<String, Value>>)`, copie-sur-écriture** (retenu). Les snapshots de rollback clonent un `Arc` (O(1)) ; une modification ne copie que la collection touchée. `BTreeMap` donne un ordre déterministe (saves et affichage stables).
2. **Structures persistantes (`im`).** Écarté pour l'instant : dépendance supplémentaire, gain réel seulement sur des collections de milliers d'éléments, improbables en VN. Le choix d'`Arc` laisse la porte ouverte (type interne).
3. **Variables « tableau » à taille fixe.** Écarté : ne couvre pas l'inventaire, et l'interface serait aussi complexe.
* Typage : listes **homogènes** déclarées (`List(Int)`, `List(Text)`…), maps à clés texte et valeurs homogènes. Pas d'imbrication en v11 (liste de maps refusée par la validation) : couvre inventaire, flags par personnage, journal ; l'imbrication est un ajout additif ultérieur.

### Ouverture d'écran depuis l'histoire

1. **Nœud `OpenScreen { screen, wait, result }`** (retenu, plan UI-14).
2. **Commande de scène.** Écarté : les commandes de scène ne bloquent pas le flux et n'ont pas de valeur de retour.
3. **Action `Custom` + script.** Écarté : contraire au no-code.

## Décisions

### 1. Valeurs liste/map (UI-13)

* `orbit_core::Value` gagne `List` et `Map` ; `VarType` gagne `List(Box<VarType>)` / `Map(Box<VarType>)` (élément scalaire seulement).
* `VarOp` (`OpKind`) gagne, filtrés par type (`OpKind::for_type`) : `Push`, `Remove` (valeur), `RemoveAt`, `Clear`, `MapSet`, `MapRemove`. `Cmp` gagne `Contains`, `NotContains`, `LenCmp`, `HasKey`. Le langage d'expressions gagne littéraux `[a, b]` / `#{k: v}`, indexation `x[i]` / `x["k"]`, `len(x)`, `contains(x, v)` ; indice hors bornes ⇒ `EvalError::IndexOutOfBounds` (même traitement que la division par zéro : no-op + avertissement sur `SetVar`).
* Limites : 10 000 éléments par collection, 64 Ko de texte total par variable ; dépassement ⇒ opération refusée + avertissement (protège snapshots et saves).
* Éditeur : déclaration de variable avec type « liste de… » / « table de… », valeur initiale éditée en tableau ; affichage dans le débogueur de variables.
* Saves : `postcard`/RON des variantes nouvelles, ajoutées **en fin d'énumération** (indices de variantes existants inchangés : une save ancienne se décode à l'identique).

### 2. `NodeKind::OpenScreen` (UI-14)

* `OpenScreen { screen: ScreenId, wait: bool, result: Option<VarName>, params: Vec<(String, Operand)> }`.
  * `wait: false` : l'écran s'ouvre, le flux continue (typique d'un HUD).
  * `wait: true` : le VM s'arrête sur le nœud jusqu'à fermeture ; l'action `UiAction::Close` gagne une valeur facultative `Close { result: Option<Operand> }` (forme sans valeur inchangée), rangée dans `result` (contrôle de type, coercion v7). Fermeture par Échap / `on_cancel` ⇒ pas de valeur, variable inchangée.
  * `params` : valeurs copiées dans des variables d'écran locales (`BindPath::Param`) avant affichage — un même écran « téléphone » peut afficher différents contacts.
* `CloseScreen { screen }` : nœud symétrique (fermer un HUD).
* **État déclaratif** : le snapshot VM gagne `open_screens: Vec<OpenScreenState { screen, params, waiting }>`. Rollback avant le nœud ⇒ l'écran disparaît ; rollback sur un nœud `wait` ⇒ l'écran est rouvert dans son état initial. L'état interne d'un écran (défilement, focus) n'est pas sauvegardé : il est reconstruit.
* **Skip** : un `OpenScreen { wait: true }` arrête le skip (c'est une décision du joueur, comme un choix). Auto-avance : idem.
* **Historique / rollback dans l'écran** : les `VarSet` faits depuis un écran ouvert par l'histoire suivent la règle v7 (reportés dans le point de retour courant, coupent le futur connu).

### 3. Écrans non bloquants (HUD)

* `ScreenAsset` gagne `hud: bool` (défaut `false`). Un écran `hud` est dessiné au-dessus du stage et **sous** la textbox et les écrans modaux ; il ne prend pas le focus clavier/manette par défaut et laisse passer les clics hors de ses widgets interactifs. `pauses_game` d'un HUD est forcé à `false` (avertissement de validation si `true`).
* Masqué automatiquement par « cacher l'interface » et pendant les écrans système (pause, sauvegarde). Plusieurs HUD peuvent coexister, ordonnés par ouverture.

### 4. Listes générées depuis une variable (UI-15)

* `WidgetKind::Repeat { source: BindPath, template: WidgetDef, empty: Option<WidgetDef> }` ; dans le gabarit, `BindPath::Item` / `ItemIndex` / `ItemKey`. Les actions du gabarit peuvent référencer l'élément (`Operand::Item`) : « utiliser l'objet », « appeler ce contact ».
* Repose sur la `ScrollList` clippée (UI-11). Plafond d'instanciation (500 éléments, avertissement au-delà).

### 5. Transitions (UI-16)

* `ScreenAsset::transition_in/out: Option<Transition { kind: Fade | Slide(dir) | Scale, duration, curve }>` (courbes M5a). Purement visuelles : l'état logique (ouvert/fermé) change immédiatement, la transition est un clip déclaratif ⇒ rollback et skip la coupent sans effet de bord. Réduction des animations (M7-07) ⇒ durée 0.

### 6. Schéma et formats

* **Schéma v11** (UI-13) : `Value::List/Map`, `ValueType` étendu, variantes `Expr` / `OpKind` / `Cmp`. Tous additifs ; migration v10 → v11 = changement de numéro.
* **UI-14** (`OpenScreen` / `CloseScreen`) : schéma suivant (v12, sauf regroupement avec une autre tâche schéma de la même file).
* **`ui_version` +1** : `hud`, `Close { result }`, `Repeat`, `BindPath::{Param, Item…}`, transitions. Les anciennes formes restent lues (`Close` sans valeur).
* `GamePack` : inchangé structurellement (l'histoire et l'UI y sont déjà sérialisées), version incrémentée seulement si l'encodage binaire de l'histoire l'exige.

### 7. Compatibilité ascendante

* Projet ≤ v10 : aucune collection ; migration triviale, fixture `compat/v10` dans la chaîne standard (v1 → v11 : charge, valide, aller-retour RON ; la v10 se joue, se sauvegarde et se recharge).
* Saves : variables scalaires décodées à l'identique (variantes ajoutées en fin). Test : save au format 4 figée avant les collections (`compat/save_v4`) chargée, rejouée, rollback. UI-14 : `open_screens` absent ⇒ `#[serde(default)]` vide.
* Un runtime antérieur ne lit pas un jeu qui utilise des collections : refus propre (variante postcard inconnue ⇒ erreur « corrompu », jamais de panique ni de corruption).

## Conséquences

* Plan : UI-13 est livré seul au schéma v11 ; UI-14 suit dans la même file schéma ; UI-14 inclut `CloseScreen`, `params` et `Close { result }` ; UI-15 inclut `Operand::Item`.
* M7-05 (succès) et M7-03 (galerie) peuvent stocker leurs ensembles dans des maps persistantes.
* Risque suivi : taille des snapshots — mesurée par un test de charge (1 000 pas de rollback sur une liste de 1 000 éléments) dans UI-13.

## Addendum : implémentation UI-13 (schéma v11)

Écarts et précisions par rapport aux décisions ci-dessus, avec leur raison.

* **Types.** `ValueType` reste `Copy` (utilisé partout par valeur) : `ValueType::List(ScalarType)` / `Map(ScalarType)`, où `ScalarType { Bool, Int, Float, Str }` est le type d'élément. L'interdiction de l'imbrication est donc structurelle dans le typage. *Écarté* : `List(Box<VarType>)` (aurait cassé `Copy` dans ~70 usages pour une imbrication que l'on refuse de toute façon).
* **Valeurs auto-typées.** `Value::List(ScalarType, Arc<Vec<Value>>)` et `Value::Map(ScalarType, Arc<BTreeMap<String, Value>>)` portent leur type d'élément : une liste vide sait ce qu'elle contient, la déclaration de variable (`VarDecl::initial`) suffit à typer (pas de nouveau champ), et une save reste auto-descriptive. *Écarté* : un champ `ty` dans `VarDecl` (surface de schéma en plus, et une valeur vide en save ne serait plus typée). RON : `List(Str, [Str("épée")])`. `serde` gagne la feature `rc` (sérialisation d'`Arc` = celle du contenu).
* **Expressions.** Variantes ajoutées en fin d'`Expr` : `List(Vec<Expr>)`, `Map(Vec<(String, Expr)>)`, `Index`, `Call(Func, Vec<Expr>)`. Fonctions `Func` : `len`, `contains` (liste ; sous-chaîne pour un texte), `has_key`, `push`, `remove` (première occurrence, sans effet si absente), `remove_at`, `insert`, `remove_key` (sans effet si absente), `clear`. Les fonctions qui modifient renvoient une nouvelle collection (copie-sur-écriture via `Arc::make_mut`) que `SetVar` range : le langage reste sans effet de bord. Texte : `[1, 2]`, `#{"clé": 1}`, vides typés `[:int]` / `#{:string}`, vide non typé `[]` / `#{}` (prend le type déclaré de la variable au rangement, coercion v7 étendue aux éléments). Un littéral dont tous les éléments sont des constantes d'un même type est replié en `Expr::Lit` (forme canonique, comme `-3`), ce qui rend impression et analyse inverses. Le DSL (M8-06) suit la même syntaxe (`#{` n'y ouvre plus un commentaire).
* **Erreurs.** `EvalError` gagne `IndexOutOfBounds { index, len }`, `MissingKey`, `TooLarge` ; `EvalError::is_recoverable()` regroupe ces cas et la division par zéro : un `SetVar` laisse la variable inchangée avec un avertissement (comme la division par zéro), une condition reste une erreur. Les limites (10 000 éléments, 64 Ko de texte) sont vérifiées à chaque construction / ajout et par la validation de la valeur initiale (`ValidationError::InvalidVariableValue`, aussi pour une collection hétérogène).
* **Opérations sans code.** `OpKind` gagne `Push`, `Remove`, `RemoveAt`, `Clear`, `MapSet`, `MapRemove` ; `VarOp` gagne `key: Option<Operand>` (`#[serde(default)]`, omis s'il est absent : les écrans UI existants se relisent à l'identique). `OpKind::operand_type` / `uses_key` disent ce que l'éditeur doit proposer. `Cmp` gagne `Contains`, `NotContains`, `HasKey`, `NotHasKey` et `LenEq…LenGe` (la proposition `LenCmp` unique aurait demandé un second opérateur dans la ligne ; six variantes s'intègrent telles quelles au menu des comparaisons). Tout se traduit en expressions (`contains(sac, "clé")`, `len(sac) > 2`) et se reconnaît en retour (`from_expr`).
* **Saves et paks : pas de changement de version.** Les nouvelles variantes étant ajoutées en fin d'énumération, l'encodage postcard des données existantes est inchangé : format de save 4 et `GamePack` 3 conservés (un bump aurait fait refuser par un runtime antérieur même les saves sans collection, sans rien apporter aux anciennes données). Garde-fous : fixtures figées `compat/save_v4/save.orbs` et `compat/pak_v3/game.pack` écrites par le build précédent, chargées par les tests. Les données persistantes (RON) lisent les deux formes.
* **Rollback.** Test de charge : 1 000 pas de rollback sur des listes de 1 000 éléments ; une liste inchangée est partagée (`Arc::ptr_eq`) par toutes les checkpoints.
* **Éditeur / MCP.** Panneau Définitions : types « Liste de … » / « Table de … (par clé) » et valeur initiale éditée en tableau (ajout, suppression, renommage de clé). Opérations et conditions sans code filtrées par type (clé de table, indice, élément). MCP : `declare_variable` accepte tableaux et objets JSON (+ `element_type` pour les vides), les vues renvoient les collections en JSON. Débogueur (`preview_state`) : collections en JSON. Liaisons UI : une collection s'affiche comme ses éléments séparés par des virgules (`Display`), la génération de widgets par élément reste UI-15.

## Addendum : implémentation UI-14 / UI-15 / UI-16 (schéma v12, saves format 5, `ui_version` 5)

Écarts et précisions par rapport aux décisions ci-dessus, avec leur raison.

* **Nœuds.** `NodeKind::OpenScreen { screen, wait, result, params: Vec<ScreenParam { name, value: Operand }> }` et `CloseScreen { screen }`, ajoutés en fin d'énumération (schéma v12, migration v11 → v12 = numéro seul, fixture figée `compat/v11`). `ScreenParam` (struct nommée) plutôt qu'un tuple : RON lisible (`(name: "contact", value: Lit(Str("Aiko")))`). Rouvrir un écran déjà ouvert le remplace. Au plus 16 écrans ouverts (le plus ancien non bloquant est fermé au-delà). Validation : écran vide, paramètre sans nom / en double / contextuel, variable de résultat non déclarée ⇒ `ValidationError::InvalidScreenNode` ; un `OpenScreen { wait: true }` compte comme point bloquant pour la détection de boucles. Un écran absent du projet est fermé par le shell avec un avertissement (le jeu ne reste jamais bloqué) ; le panneau Problèmes le signale.
* **VM.** `VmState::open_screens: Vec<OpenScreenState { screen, node, params (évalués), waiting }>` ; `Pending::Screen { node }`, `Interaction::Screen { node, screen }`, `StepResult::Screen { node, screen }` (variantes ajoutées en fin). Un écran bloquant est un point de rollback comme une ligne ou un choix. `Vm::close_screen(screen, Option<Value>)` range la valeur dans `result` (type déclaré conservé par coercion v7, sinon avertissement et variable inchangée) ; fermer l'écran attendu se comporte comme répondre à un choix (un pas de rollback le rouvre, le futur connu est coupé) ; fermer un écran non bloquant est reporté dans le point de retour courant (comme un `VarSet` depuis un écran). Un `SceneGoto` ferme l'écran attendu, les HUD restent. « Jouer d'ici » (`seek`) ferme les écrans bloquants sans valeur.
* **Saves : format 5.** Le champ `open_screens` est ajouté à la fin de `VmState` : la disposition postcard change, d'où le format 5 ; `legacy_v4` décode les saves du format 4 (fixture figée `compat/save_v4`) sans écran ouvert, ce qui est exact. Pak : version 3 inchangée (variantes ajoutées en fin ; l'UI y est stockée en RON). Le hash de structure de l'histoire gagne les étiquettes 24 / 25.
* **`Close { result }` ⇒ `UiAction::CloseWith(Operand)`.** Transformer la variante unitaire `Close` en variante à champ aurait cassé la lecture RON (`Close` sans parenthèses) et l'encodage des documents existants ; une variante ajoutée garde `Close` intact. L'éditeur présente une seule action « Fermer » avec une valeur de retour facultative. Sur un écran que l'histoire n'a pas ouvert, `CloseWith` est un `Close`.
* **Opérandes contextuels.** `Operand` gagne `Item`, `ItemIndex`, `ItemKey`, `Param(String)` (fin d'énumération) ; ils s'évaluent comme les noms réservés `@item`, `@index`, `@key`, `@param.nom`, qu'un écran fournit à sa recherche de variables (aucune variable ne commence par `@`) ; ailleurs ce sont des variables inconnues. Les boutons d'une ligne générée reçoivent les valeurs résolues à la construction (le bouton se souvient de son élément).
* **HUD.** `ScreenAsset::hud` ; couche `Layer::UI - 1` (sous la textbox, au-dessus du stage) ; jamais de pause (`effective_pauses_game`), pas de focus clavier ; un appui sur un de ses contrôles lui revient (et le relâchement qui suit), le reste va au jeu. Masqué pendant « cacher l'interface » et dès qu'un menu est ouvert. Un écran de l'histoire qui n'est pas un HUD prend toute l'entrée (sauf les clics sur le menu rapide, pour garder le rollback) ; Échap exécute son `on_cancel`, ou ouvre la pause s'il n'en a pas.
* **Listes générées (UI-15).** `WidgetKind::Repeat { source, dir, empty }` : les **enfants** du widget sont le gabarit d'une ligne (plutôt qu'un champ `template` séparé : l'arbre de l'éditeur d'interface les édite sans rien ajouter) ; `dir` (`Grid(n)` pour une grille) ; `empty` = texte affiché sans élément ; 500 lignes au plus. Source : variable ou paramètre liste/map (une map donne ses entrées par ordre de clé). Les gabarits peuvent s'imbriquer (la ligne intérieure masque l'extérieure), pas les collections. Les conditions `visible` / `enabled` ne lisent pas encore l'élément (ajout ultérieur possible).
* **Transitions (UI-16).** `ScreenAsset::transition_in / transition_out: Option<ScreenTransition { kind: Fade | SlideUp | SlideDown | SlideLeft | SlideRight | Scale, duration, ease: Option<Curve> }>` ; une courbe `Named` est cherchée dans les courbes de l'histoire. Appliquées après coup aux sprites dessinés par l'écran (opacité, décalage, échelle autour du centre) : aucun widget n'en sait rien. Écrans de l'histoire et écrans de données de la pile ; rollback, chargement et réduction des animations les coupent.
* **`ui_version` 5** (la 4 a été prise par M7-02) : migration v4 → v5 = numéro seul ; fixtures figées `compat/ui_v3/journal.screen.ron` et `compat/ui_v4/journal.screen.ron`. Les champs nouveaux ne sont pas écrits à leur valeur par défaut : un document ancien réécrit ne change que de numéro.
