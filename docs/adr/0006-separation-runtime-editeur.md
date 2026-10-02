# ADR 0006 : Séparation runtime / éditeur et règles de CI

Statut : Accepté

## Contexte
Le jeu exporté ne doit embarquer ni l'éditeur ni egui. La logique VN doit rester testable sans GPU et compilable en wasm, pour que le web ne soit jamais une surprise tardive.

## Décision
- Le player ne dépend jamais de l'éditeur, d'egui, ni du serveur MCP.
- Les crates core (`orbit_core`, `orbit_project` au M0 ; plus tard `orbit_story`, `orbit_vm`, `orbit_save`, `orbit_i18n`) n'ont aucune dépendance graphique ou plateforme, compilent en `wasm32-unknown-unknown` et se testent sans GPU.
- Pas de `std::thread`, `std::fs` ni `Instant` direct hors de `orbit_platform` (temps via `web-time`).
- `orbit_gfx` et `orbit_platform` compilent aussi en wasm (WebGL2).
- Appliqué par `cargo xtask check-arch` (analyse de `cargo tree` sur toutes les cibles, plus build wasm32) et par la CI (jobs Windows, Linux, wasm, cargo-deny). `cargo deny` n'autorise que des licences permissives.

## Conséquences
- Toute nouvelle dépendance de `orbit_core` / `orbit_project` est vérifiée automatiquement.
- La liste des crates interdites dans `xtask/src/arch.rs` doit être tenue à jour (egui, éditeur, MCP).
