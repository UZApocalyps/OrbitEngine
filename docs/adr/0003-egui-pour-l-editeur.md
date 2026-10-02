# ADR 0003 : egui pour l'éditeur, UI en jeu maison

Statut : Accepté

## Contexte
L'éditeur est une application riche (docks, graphe de nœuds, inspecteurs, timeline) qui doit partager `orbit_platform` et `orbit_gfx` avec le player. L'UI en jeu d'un VN demande un habillage artistique que les toolkits d'outils ne fournissent pas.

## Décision
- Éditeur : `egui` avec `egui-wgpu` et `egui-winit`, `egui_dock` pour les panneaux, `egui-snarl` pour le graphe. La preview est le vrai runtime rendu dans une texture wgpu affichée dans un panneau egui (WYSIWYG exact).
- UI en jeu : widgets et thèmes maison (`orbit_ui`), rendus par `orbit_render2d`, data-driven (RON) et éditables dans l'éditeur. egui n'est jamais utilisé en jeu.

## Conséquences
- Itération rapide sur l'éditeur, un seul pipeline de rendu.
- Le player ne dépend jamais d'egui ni de l'éditeur (vérifié par `cargo xtask check-arch`).
- Un système de widgets en jeu est à écrire (M2).
