# ADR 0005 : Stratégie de scripting reportée au M8

Statut : Accepté

## Contexte
Le public cible est no-code : tout doit être faisable depuis l'éditeur. Le scripting n'est nécessaire ni au MVP ni aux jalons 3D, mais ne doit pas être bloqué par des choix précoces.

## Décision
- Scripting reporté au M8 : Rhai (Rust pur, compatible wasm) pour conditions, variables et logique, plus plugins Rust natifs compilés statiquement (`orbit_plugin_api`), sans dylib.
- Invariants posés dès le M0 : l'état de jeu n'est modifiable que via l'API du VM (store snapshotté) ; le RNG est seedé et sauvegardé ; tout effet externe futur doit être réversible ou marqué non annulable (rollback).
- C# ou plugins WASM : backlog desktop uniquement, sans engagement.

## Conséquences
- L'IR du M0 ne contient que des conditions et affectations simples (`SetVar`, `If`).
- Les crates VM (M1+) devront respecter ces invariants pour que Rhai s'ajoute sans refonte.
