# ADR 0016 : Écrans de gameplay, HUD et valeurs liste/map

Statut : proposé (UI-13 et UI-14, à relire avant implémentation). Complète ADR 0011 et ses addenda.

## Contexte

Les écrans data-driven (ADR 0011) ne s'ouvrent que depuis d'autres écrans ; l'histoire ne peut pas afficher une carte, un téléphone ou un inventaire, ni un HUD permanent (jauge, or). Les variables n'ont que des scalaires (entier, décimal, booléen, texte) : un inventaire ou une liste de contacts demande des collections. Les deux besoins arrivent ensemble au **schéma v10** (plan §9). Contraintes : rollback, skip, saves exacts ; compatibilité ascendante (§1.3) ; tout faisable sans code (§1.4).

## Options examinées

### Collections

1. **`Value::List(Arc<Vec<Value>>)` et `Value::Map(Arc<BTreeMap<String, Value>>)`, copie-sur-écriture** (retenu). Les snapshots de rollback clonent un `Arc` (O(1)) ; une modification ne copie que la collection touchée. `BTreeMap` donne un ordre déterministe (saves et affichage stables).
2. **Structures persistantes (`im`).** Écarté pour l'instant : dépendance supplémentaire, gain réel seulement sur des collections de milliers d'éléments, improbables en VN. Le choix d'`Arc` laisse la porte ouverte (type interne).
3. **Variables « tableau » à taille fixe.** Écarté : ne couvre pas l'inventaire, et l'interface serait aussi complexe.
* Typage : listes **homogènes** déclarées (`List(Int)`, `List(Text)`…), maps à clés texte et valeurs homogènes. Pas d'imbrication en v10 (liste de maps refusée par la validation) : couvre inventaire, flags par personnage, journal ; l'imbrication est un ajout additif ultérieur.

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

* **Schéma v10** : `Value::List/Map`, `VarType` étendus, nœuds `OpenScreen` / `CloseScreen`, variantes `OpKind`/`Cmp`. Tous additifs ; migration v9 → v10 = changement de numéro.
* **`ui_version` +1** : `hud`, `Close { result }`, `Repeat`, `BindPath::{Param, Item…}`, transitions. Les anciennes formes restent lues (`Close` sans valeur).
* `GamePack` : inchangé structurellement (l'histoire et l'UI y sont déjà sérialisées), version incrémentée seulement si l'encodage binaire de l'histoire l'exige.

### 7. Compatibilité ascendante

* Projet ≤ v9 : aucune collection ni `OpenScreen` ; migration triviale, fixture `compat/v9` et test standard.
* Saves ≤ v9 : `open_screens` absent ⇒ `#[serde(default)]` vide ; variables scalaires décodées à l'identique (variantes ajoutées en fin). Test : save v9 chargée en v10 puis rollback.
* Un runtime antérieur ne lit pas un jeu v10 : refus propre par numéro de schéma.

## Conséquences

* Plan : UI-13 et UI-14 restent groupés sur v10 (UI-13 d'abord, même file schéma) ; UI-14 inclut `CloseScreen`, `params` et `Close { result }` ; UI-15 inclut `Operand::Item`.
* M7-05 (succès) et M7-03 (galerie) peuvent stocker leurs ensembles dans des maps persistantes.
* Risque suivi : taille des snapshots — mesurée par un test de charge (1 000 pas de rollback sur une liste de 1 000 éléments) dans UI-13.
