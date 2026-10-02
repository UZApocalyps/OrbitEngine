# ADR 0004 : IR RON canonique et identifiants ULID

Statut : Accepté

## Contexte
Sauvegardes, localisation, diffs, undo et pilotage par IA exigent des références stables vers les éléments d'une histoire. Un format textuel versionné facilite le diff et la revue.

## Décision
- Le modèle canonique est l'IR Rust (`orbit_core`) sérialisée en RON, un fichier par scène/histoire. Un DSL texte éventuel (M8) n'est qu'une vue import/export.
- Chaque nœud, scène et chaîne localisable porte un identifiant stable ULID (newtypes `NodeId`, `SceneId`, `StringId`), sérialisé en chaîne de 26 caractères. Les sauts ciblent des `Label` par id, jamais par nom ou position.
- Chaque document porte un `schema_version`. Les migrations opèrent sur le texte RON, une version à la fois (`orbit_core::migrate`) ; la table est vide au M0 (version 1).
- La validation (ids dupliqués, cibles inexistantes ou non-Label, choix vide) retourne des erreurs typées.
- `orbit build` compilera le RON en pack binaire (`postcard`) plus tard. Les positions du graphe vivent dans un sidecar `.layout`.

## Conséquences
- Les ids sont générés avec `web-time` et `getrandom` (compatible wasm) ; `ulid` est utilisé sans ses features `std`/`serde`, la sérialisation étant écrite à la main.
- Toute évolution de schéma doit s'accompagner d'une migration, sinon les anciens fichiers deviennent illisibles.

## Mise à jour M1 : schéma version 2
- Nouveaux ids ULID : `CharacterId`, `AssetId`. Les nœuds ne référencent jamais un chemin : les assets passent par un manifest (`assets`, id -> chemin relatif + type) stocké dans le fichier d'histoire.
- Table de chaînes (`strings`) : tous les textes affichés (répliques, options, noms de personnages) ont un `StringId` ; la langue source (`source_lang`) est stockée avec la table. Les autres langues seront des tables séparées indexées par les mêmes ids (non implémenté).
- Expressions typées (`Expr`) pour les conditions et `SetVar`, avec parser/afficheur texte optionnel (`Expr::parse`).
- Validation : retourne un rapport `erreurs + avertissements` (variables non définies, nœuds inatteignables, boucles sans attente, choix tous conditionnels). L'existence des fichiers d'assets se vérifie à part (`check_asset_files`), pour rester sans I/O.
- Migration 1 -> 2 (`migrate.rs`) : les noms de locuteurs deviennent des personnages, les textes passent dans la table de chaînes, les conditions deviennent des expressions.
