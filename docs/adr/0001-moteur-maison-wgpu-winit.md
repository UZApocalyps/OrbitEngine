# ADR 0001 : Moteur maison sur wgpu + winit

Statut : Accepté

## Contexte
OrbitEngine vise un éditeur no-code, des animations 2D, de la 3D composée dans l'éditeur et des exports Windows, Linux et Web (WebGL2), Android plus tard. Un framework moteur (Bevy, etc.) impose son cycle de release, ses changements d'API et son modèle de données, ce qui pèse sur l'éditeur, l'undo et la sérialisation.

## Décision
Aucun framework moteur. Le moteur est assemblé à partir de bibliothèques briques, chacune remplaçable derrière une de nos crates : `wgpu` (rendu), `winit` (fenêtre et input), `glam`, `cosmic-text`, `kira`, `image`, `gltf`, `web-time`, `wasm-bindgen`. Le rendu est conçu par couches : `orbit_gfx` (bas niveau) puis `orbit_render2d` / `orbit_render3d`.

Versions épinglées au M0 : `wgpu =30.0.1` (WebGL2 via la feature `webgl`, WebGPU navigateur volontairement non activé), `winit =0.30.13` (API `ApplicationHandler`), `wasm-bindgen =0.2.129` (le CLI doit avoir exactement la même version).

## Conséquences
- Contrôle total, aucune attente de fonctionnalités tierces.
- Coût : renderer 2D/3D, gestion d'assets, boucle de jeu et abstraction plateforme à écrire soi-même.
- Le web (WebGL2) est testé en CI dès le M0 : limites `downlevel_webgl2_defaults`, pas de compute, pas de `base_vertex`.
- Les mises à jour de `wgpu` et `winit` sont des décisions explicites (versions épinglées).
