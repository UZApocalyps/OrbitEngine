# ADR 0002 : Graphe de scène maison plutôt qu'ECS (hecs)

Statut : Accepté

## Contexte
La ROADMAP demande de trancher au M0 entre un ECS et un graphe de scène. Un visual novel manipule quelques dizaines d'entités (fond, sprites, textbox, widgets). L'éditeur doit offrir undo/redo, inspecteur auto-généré et sérialisation lisible.

## Décision
Un graphe de scène maison simple (hiérarchie de nœuds avec transforms, données déclaratives), dans `orbit_scene` (indépendant du GPU). Pas d'ECS, pas de `hecs`, pas de scheduler. Si un stockage dense devient nécessaire, `hecs` pourra être utilisé comme simple conteneur derrière l'API de `orbit_scene`, sans changer le modèle de données.

## Conséquences
- Modèle de données direct à sérialiser en RON, à inspecter et à modifier par commandes (undo/redo).
- Pas d'optimisation de masse d'entités : acceptable pour le périmètre VN.
- Aucune crate n'est créée au M0 : la décision cadre `orbit_scene` (M1+).
