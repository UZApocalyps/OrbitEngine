# À faire plus tard

Idées validées mais hors des jalons en cours. À replanifier dans la roadmap le moment venu.

## Variables sans code (UI phase 2)
- **Fait** (voir ADR 0011, addendum « Variables sans code ») : `VarOp` / `Condition` dans `orbit_core`, schéma v7, `Vm::apply_var_op`, `UiAction::VarSet`, `Cond::Vars`, liaisons `Var`, widgets de l'éditeur (nœuds `SetVar` / `If` / choix et éditeur d'interface), mode expression conservé.
- Reste : `UiAction::SceneGoto` (le VM n'a pas d'aller-à-une-scène hors nœud) ; liste déroulante liée à une variable ; condition « variable A comparée à une expression » ; avertissement de division par zéro dans le panneau Problèmes pour les nœuds `SetVar` en mode expression.

## Personnages animés
- **Fait (schéma v8, ADR 0004 addendum)** : image fixe ou animation par expression (sprite sheet, suite d'images, fps, boucle / une fois / aller-retour), runtime et éditeur (panneau Personnages, aperçu animé de la miniature, aperçu de jeu).
- Reste : animation dans la vue de scène de l'éditeur (elle montre la première image), import de GIF / APNG, sprite sheet à cellules de tailles inégales (atlas), animations sur les anciens écrans (portraits, UI), saut d'images selon le temps réel lors d'un retard.
- Plus tard : animation rigged (squelette, type Spine/Live2D-like) : la variante `Motion::Rigged` est réservée.


## Moderniser le thème de base du jeu
- Le thème par défaut d'`orbit_ui` (menus, textbox, quick menu, écrans save/load/préférences) est fonctionnel mais daté : refonte visuelle complète.
- Pistes : coins arrondis / 9-slice, ombres douces, flou ou dégradés, animations d'apparition/survol, typographie et espacements revus, icônes, mise en page responsive (16:9, 4:3, portrait mobile).
- Corriger au passage : quick menu qui chevauche la textbox, mesure de texte approximative (pas de clipping des ScrollList).
- Plusieurs thèmes prêts à l'emploi sélectionnables depuis l'éditeur.
- **Éditeur de thème de l'UI en jeu** (repoussé hors de M3e) : inspecteur auto-généré sur le `Theme` RON d'`orbit_ui` avec preview en direct ; à faire avec cette refonte, pas avant.

## Éditeur de courbes d'easing pour les transitions
- Éditeur visuel de courbes (Bézier cubique + presets : linear, ease-in/out, back, bounce, elastic…) avec aperçu animé.
- Applicable aux transitions (fade, dissolve, mask dissolve) et, à terme, aux tweens/animations M5 (même composant réutilisé dans la timeline).
- Courbes nommées réutilisables stockées dans le projet ; sérialisées dans l'IR, exécutées à l'identique en natif et web.
- Exposé au MCP (création/assignation de courbes).

## Extension de fichier dédiée aux histoires (`.orbit`)
- Remplacer `.ron` par `.orbit` pour les fichiers d'histoire (le contenu reste du RON) ; `.ron` toujours accepté pour la compatibilité.
- Manifeste, traductions, thèmes et préférences restent en `.ron` (ou `.orbitproj` pour le manifeste, à décider).
- Association de fichier Windows/Linux dans les installeurs (double-clic → éditeur, icône dédiée).
- Décision (2026-10-02) : validé, planifié en TL-02 de [IMPLEMENTATION_PLAN](IMPLEMENTATION_PLAN.md).

## Optimisations
- Taille du bundle web : fait au M4 (10,2 → 4,4 Mo, budget en CI). Pistes restantes : décodage JPEG / polices chargés à la demande, `cosmic-text` sans `swash` inutile, binaryen récent, `twiggy` pour chercher les gros morceaux.
- Profilage CPU/GPU du runtime et de l'éditeur.

## Dialogues en bulles positionnables
- Mode d'affichage alternatif à la textbox classique : chaque réplique est rendue dans une **bulle** (style BD / conversation) plutôt que dans la boîte du bas.
- Le créateur positionne les bulles depuis l'éditeur, sans code : par personnage (bulle ancrée au personnage, suit sa position) ou par réplique (position libre surchargée).
- Réglages : forme et style de la bulle (thème), queue orientée vers le locuteur, taille max / retour à la ligne auto, empilement ou remplacement des bulles précédentes (effet conversation à deux).
- Doit rester compatible avec : machine à écrire, historique, rollback/skip (état des bulles déclaratif), localisation (taille variable du texte), et le MCP (outils de placement).
- Dépend de : `orbit_ui` (thèmes), éditeur M3 (placement visuel), M5 (animation d'apparition des bulles).

## Auto-update de l'éditeur
- Mise à jour automatique de l'éditeur installé (`cargo-packager-updater` : manifeste signé hébergé avec les releases GitHub). Non nécessaire pour un moteur à gros jalons ; à reprendre avec la signature de code Windows et le projet démo copié dans le dossier utilisateur.
