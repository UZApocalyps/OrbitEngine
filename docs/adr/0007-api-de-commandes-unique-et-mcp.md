# ADR 0007 : API de commandes unique et serveur MCP

Statut : Accepté

## Contexte
L'éditeur, la CLI et une IA (via MCP) doivent modifier un projet de façon validée, annulable et observable, sans jamais écrire directement dans les fichiers RON.

## Décision
- Invariant dès le M0 : toute modification d'un projet passe par l'API de commandes d'`orbit_project`. `Project` n'expose aucun accès mutable à la `Story`.
- Une `Command` typée s'applique et produit son inverse. Les inverses alimentent une pile undo/redo ; une transaction regroupe plusieurs commandes en une seule entrée d'undo et est atomique (rollback complet en cas d'échec).
- Les changements émettent des événements (`ChangeEvent`) auxquels on s'abonne.
- Chaque nœud a une version monotone. Une commande peut porter une `expected_version` : si le nœud a changé entre-temps, elle échoue (`VersionConflict`) sans rien modifier.
- Un futur `orbit_mcp` (SDK `rmcp`, licence à vérifier via `cargo deny`) exposera cette API : headless (stdio) puis live dans l'éditeur, désactivé par défaut, localhost, jeton, périmètre limité au projet, jamais dans le player.

## Conséquences
- Éditeur et IA partagent validation, undo et file de commandes.
- Les schémas d'outils MCP seront générés depuis les types de l'IR.
- Le M0 ne contient que trois commandes (`InsertNode`, `RemoveNode`, `SetSay`) ; le catalogue s'étend avec l'éditeur (M3).
