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

## Mise à jour M3d : schéma version 5, les scènes ne s'enchaînent plus
- Jusqu'à la v4, une scène dont le dernier nœud n'avait pas de saut « tombait » dans la suivante (la liste des scènes était parcourue à plat). Ce comportement implicite rendait le graphe de l'histoire illisible (les arêtes dépendaient de l'ordre des scènes) et dangereux à l'édition (réordonner ou ajouter une scène changeait le déroulement).
- v5 : **une scène ne se poursuit jamais dans la suivante.** On quitte une scène uniquement par `NodeKind::JumpScene { scene, label }` (début de la scène cible, ou `Label` de cette scène), ou par un `Jump` / `Call` / option vers une étiquette. Une scène dont l'exécution atteint la fin sans saut, `End` ni `Return` termine la partie (comme `End`).
- Validation : scène cible inexistante et étiquette absente de la scène cible sont des erreurs ; atteindre la fin d'une scène est un avertissement (« la partie se termine ici »). Un `JumpScene` vers une scène vide termine la partie (même avertissement).
- Migration 4 -> 5 : à la fin de chaque scène qui « tombait » dans la suivante non vide (dernier nœud autre que `Jump`, `JumpScene`, `Choice`, `End`, `Return`), un `JumpScene` vers cette scène est ajouté ; le comportement des histoires existantes est conservé. Les positions du graphe vivent dans `<histoire>.layout` (RON, hors de l'IR).

## Mise à jour (éditeur de vue) : schéma version 6, taille, miroir et position libre des personnages
- **Besoin** : ajuster la taille d'un personnage, l'inverser sur X ou Y, et le placer librement (au-delà de gauche / centre / droite et des positions nommées du projet), depuis un éditeur de vue. Tout est **additif** : chaque nouveau champ a une valeur par défaut et n'est **pas écrit** dans le RON quand il vaut ce défaut (`Story::to_ron` retire ces lignes ; le format binaire du pak de jeu, non auto-descriptif, écrit toujours tous les champs, sans quoi la lecture se déphaserait), donc un fichier v5 est un fichier v6 valide (la migration 5 -> 6 ne change que le numéro) et un fichier v6 qui n'utilise rien de nouveau est identique à l'ancien.
- `NodeKind::Show` gagne `scale: Option<f32>` (échelle uniforme, ou horizontale quand `scale_y` est donnée ; `None` = l'échelle par défaut du personnage), `scale_y: Option<f32>` (étirement vertical distinct ; `None` = suit `scale`), `flip_x: bool` et `flip_y: bool` (miroir sur l'axe X / Y, `false` omis du fichier).
- `Character` gagne `default_scale: f32` (1,0 par défaut, omis du fichier quand il vaut 1,0).
- `Position` gagne la variante `Free { x, y }` : coordonnées normalisées (origine en haut à gauche, sprite ancré par son bas-centre, comme les positions nommées), **sans déclaration** dans `Story::positions`. C'est ce que l'éditeur de vue écrit quand on glisse un personnage ; il réécrit une position nommée quand le glissement s'aimante exactement dessus. `Position` perd `Eq` / `Hash` / `Ord` (flottants) ; il n'en avait besoin nulle part.
- **Un `Show` décrit tout l'aspect du personnage.** Montrer un personnage déjà présent remplace son expression, sa position, son échelle et son miroir (avec la transition éventuelle), en gardant son rang d'affichage : c'était déjà le comportement du VM (mise à jour sur place), il est désormais testé pour l'échelle et le miroir. Un `Show` sans `flip_x` remet donc le personnage à l'endroit : pas de nœud « Déplacer / Modifier le personnage » séparé, ce qui garde un seul endroit où lire l'état du décor à un instant donné (le scénariste qui veut seulement bouger un personnage réaffiche le même, comme avant pour changer d'expression).
- **Validation** (erreurs) : échelle (de `Show` ou par défaut) finie et dans `0,05 ..= 10` (`InvalidScale`), coordonnées libres finies et dans `-2 ..= 3` (`InvalidFreePosition` : un personnage peut entrer en dépassant un peu du cadre).
- **VM** : `CharacterOnStage` porte l'échelle **effective** (`[x, y]`, l'échelle du `Show` sinon celle du personnage), le miroir et le `Show` d'origine (`source`, métadonnée d'éditeur) ; `PresentationEvent::Show` porte `scale`, `flip_x`, `flip_y`. Ils sont donc restaurés par le rollback, le chargement d'une sauvegarde et « jouer depuis ici ».
- **Sauvegardes** : l'état de la VM change de disposition (postcard n'est pas auto-descriptif), le format des sauvegardes passe donc à la **version 2** ; la version 1 reste lisible (décodeur dédié, `orbit_save::legacy_v1` : les personnages reviennent à l'échelle 1,0, sans miroir).
- Migration 5 -> 6 (`migrate.rs`) : numéro de version seulement (testée sur un fichier v5 : champs par défaut, rien d'écrit de plus). `examples/stories/demo.ron` est en v6 et utilise miroir, échelle et position libre.


## Addendum : personnages animés (schéma v8)

- `Expression` gagne `motion: Option<Motion>` (absent du fichier RON quand c'est `None`, donc une image fixe s'écrit comme avant). `Motion::Animation(Animation { frames, fps, loop_mode })` ou `Motion::Rigged` (**réservé** : non géré, avertissement de validation `RiggedUnsupported`, l'image fixe est dessinée). `AnimFrames::SpriteSheet { asset, cols, rows, frame_count, frame_size }` (grille lue de gauche à droite, de haut en bas ; `frame_size` optionnel pour les planches avec marges) ou `AnimFrames::Sequence(Vec<AssetId>)`. `LoopMode` : `Loop`, `Once` (garde la dernière image), `PingPong`.
- Modèle **additif** : `image` reste l'image « poster » (miniatures, repli tant que les frames ne sont pas chargées) ; l'éditeur la garde alignée sur la première image / la planche. Schéma **v8**, migration v7 -> v8 : numéro de version seulement (les images fixes restent valides).
- Validation (erreurs `InvalidAnimation`) : `fps` fini et > 0, grille d'au moins 1x1, `frame_count` entre 1 et `cols * rows`, taille de cellule >= 1x1, séquence non vide ; chaque frame doit être un asset `Image` connu.
- Calcul pur dans `orbit_core::anim` (`frame_index`, `Animation::frame_at`, `sheet_uv`, `sheet_cell_size`), partagé par le runtime et l'éditeur. Runtime : `AnimClock` (temps par personnage, qui n'avance que quand le jeu tourne : figé par un écran de pause ou pendant le chargement), sous-rectangle UV via `Sprite2d::uv` (compatible miroir, échelle et transitions), l'animation redémarre à chaque affichage ; rien dans les sauvegardes ni le rollback. L'export inclut toutes les frames (`asset_users` compte les frames).
- Format `GamePack` : `motion` est toujours écrit en binaire (comme `Character::default_position`).

## Addendum : schéma v14, expressions riggées (M5B-07)

- `Motion::Rigged(RiggedMotion { rig, skin, clip, loop_mode })` remplace la variante unitaire réservée en v8 ; `AssetKind::Rig` désigne les fichiers `.rig.ron`. Migration v13 → v14 : l'ancien `Rigged` unitaire (qui n'a jamais eu d'effet) est retiré du texte avant les étapes de migration et l'expression devient une image fixe. Fixture figée `examples/fixtures/compat/v13`. Détails : ADR 0013, addendum M5B-07.

## Addendum : extension `.orbit` des histoires (TL-02)

- **Extension.** Une histoire s'enregistre en `<nom>.orbit`. Le contenu est strictement le même RON (même `schema_version`, même migration) : ce n'est qu'un changement de nom, pas de format. Les nouveaux projets créent `story.orbit`.
- **Compatibilité (impérative).** `.ron` reste lu partout (éditeur, CLI, export, lecteur, MCP, rechargement à chaud) : un manifeste qui désigne `story.ron` continue de fonctionner tel quel, et un dossier sans manifeste cherche `story.orbit` puis `story.ron`. Aucun chemin de lecture n'est supprimé.
- **Conversion.** À l'ouverture d'un projet dont l'histoire est en `.ron`, l'éditeur affiche une notification ; l'action *Fichier > Convertir l'histoire en .orbit* enregistre, renomme le fichier (le sidecar `.layout` et les traductions `i18n/<nom>.<lang>.ron` sont nommés d'après le radical et restent valides), met à jour `story` dans le manifeste, puis rouvre le projet. Aucun renommage automatique ni silencieux ; en cas d'échec rien n'est modifié.
- **Manifeste : on ne crée pas de `.orbitproj`.** Le manifeste reste `orbit.project.ron`. Raisons : un seul nouveau suffixe à faire connaître, pas de second type de fichier à associer dans les installeurs, aucune migration de projet, et l'association du double-clic cible l'histoire, qui est ce que l'utilisateur voit (un double-clic sur `story.orbit` ouvre l'éditeur sur le projet de son dossier, via le manifeste). À reconsidérer seulement si l'on veut un jour un fichier projet unique (pak, assets inclus).
- **Reste en `.ron`** : manifeste, traductions, thèmes et écrans `ui/`, préférences, sauvegardes de recovery.
- **Installeurs.** Association `.orbit` (type MIME `application/x-orbit-story`) via `file-associations` de cargo-packager (WiX, NSIS, `.desktop` + shared-mime-info pour deb et AppImage). Pas encore d'icône de document dédiée.
- **Suite** : la démo (`examples/stories/demo.ron`) et les fixtures de test restent en `.ron` pour l'instant, elles couvrent le chemin de compatibilité ; leur renommage en `.orbit` est une tâche à part (beaucoup de tests les référencent).

## Addendum : succès et voix (schéma v16, M7-05 / M7-06)

* **Schéma v15 -> v16**, migration = numéro seul (`v15_to_v16`) : tout est additif. `Story::achievements: Vec<Achievement>` (`id: AchievementId`, `title_id` / `description_id` : `StringId` de la table de textes, donc traduisibles, `hidden`, `icon: Option<AssetId>` image) ; `NodeKind::Unlock { achievement }` (variante ajoutée en dernier) ; `NodeKind::Say` gagne `voice: Option<AssetId>` (un asset `Sound`, jamais un chemin). Les champs « toujours écrits » par `Serialize` (le pak postcard ne saute rien) sont retirés du RON lisible quand ils valent leur défaut (`voice: None,`, `achievements: [],` dans `DEFAULT_LINES`). Fixture figée `examples/fixtures/compat/v15`.
* **Validation** : `DuplicateAchievementId`, `InvalidAchievement` (texte ou icône inconnus, icône qui n'est pas une image), `UnknownAchievement` (nœud `Unlock`), voix : asset inconnu ou qui n'est pas un son ; avertissement `AchievementNeverUnlocked`.
* **Pak `GamePack` version 6 (compatibilité ascendante ET descendante).** Postcard est positionnel : les nouveaux champs (`Say::voice`, `Story::achievements`, `Translation::voices`) changent la disposition. Ils sont écrits via `serialize_with = v16_skip` / lus via `deserialize_with = v16_field` (`orbit_core::achievements`) : sous `with_legacy_v15_layout` (drapeau thread-local, comme `with_legacy_motion_layout`) ils ne produisent aucun octet (`serialize_unit`) et se lisent comme leur défaut. `GamePack::encode` n'écrit la version 6 que pour un jeu qui s'en sert (`uses_v16` : voix, succès, nœud `Unlock`, voix traduite) ; **tout autre jeu est encodé octet pour octet comme avant** (versions 3, 4 ou 5) et reste lisible par les anciens lecteurs, qui refusent proprement un jeu en version 6. Les versions 1 à 5 se lisent avec la disposition v15. Fixture figée `examples/fixtures/compat/pak_v5/game.pack` (jeu à expression animée par squelette et nœud `Script`) : rechargée, ré-encodée à l'identique.
* **Traductions** : `Translation::voices: BTreeMap<StringId, AssetId>` (clé : le texte de la réplique) remplace, pour la langue choisie, la voix du nœud ; le fichier RON ne l'écrit que s'il est non vide, `Localizer::voice(id)` la résout. Un fichier existant se lit tel quel.
* **Persistance** : voir l'addendum de l'ADR 0008. **UI** : voir l'addendum de l'ADR 0011 (`ui_version` 7).
* **Lacunes** : pas de préchargement des voix des traductions en mode asset paresseux (web) : seule la voix du nœud est préchargée avec sa scène ; pas d'icône de succès par défaut dessinée (la carte reste vide sans icône).
