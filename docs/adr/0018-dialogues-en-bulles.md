# ADR 0018 : Dialogues en bulles

Statut : proposé (TL-05, à relire avant TL-06).

## Contexte

Certains VN (style BD, téléphone, scènes de groupe) affichent les répliques dans des bulles près du personnage plutôt que dans une textbox. Contraintes : rollback, skip, historique et saves inchangés dans leur principe ; texte localisé de longueur variable ; no-code ; compatibilité ascendante.

## Options examinées

1. **Mode d'affichage de la réplique, avec héritage projet → scène → réplique** (retenu). Une réplique reste une réplique (même nœud `Say`) ; seul son rendu change.
2. **Nouveau nœud `Bubble`.** Écarté : dupliquerait voix, historique, traduction, auto-avance.
3. **Bulles comme widgets UI posés à la main.** Écarté : aucun lien avec la machine à écrire, l'historique et le rollback.

## Décisions

* `DialogueMode::{Textbox, Bubble}` : défaut projet, surcharge facultative par scène et par réplique (champs `Option`, absents = hérité). Défaut projet = `Textbox` ⇒ comportement actuel.
* **Ancrage** : `BubbleAnchor::{Character { offset }, Free { pos }}`. Ancré au personnage, la bulle suit le sprite (point d'ancrage « tête » défini sur le personnage, éditable dans la vue de scène) ; position libre en coordonnées normalisées. La queue est orientée automatiquement vers l'ancre (côté choisi pour rester à l'écran) ; `tail: None` pour une pensée/narration.
* **Placement** : la taille dépend du texte **mesuré après traduction** ; largeur max par style, retour à la ligne, puis repli (décalage pour rester dans l'écran, sinon bascule en textbox avec avertissement d'export si une langue déborde).
* **Empilement** : `stack: Replace | Keep(n)` par scène — `Replace` efface la bulle précédente ; `Keep(n)` garde les n dernières, les plus anciennes estompées (conversation de groupe). L'ensemble visible est **dérivé** des n dernières répliques du chemin parcouru : rien de nouveau dans le snapshot, donc rollback, skip et saves exacts par construction.
* **Rendu** : style UI `bubble` (9-slice UI-12, queue en sprite séparé), machine à écrire et voix identiques à la textbox, historique inchangé (les répliques y figurent normalement). Accessibilité : annoncées comme les répliques de textbox (ADR 0017).
* **Schéma** : champs optionnels sur projet, scène et `Say` ; additifs, au prochain numéro libre (après v12). Ancien projet = tout `None` = textbox.

## Conséquences

* TL-06 : rendu, placement par glisser dans la vue de scène, inspecteur, MCP, tests rollback/skip/traduction longue.
* Dépend de UI-12 (9-slice) et M5A (courbes pour l'apparition).
