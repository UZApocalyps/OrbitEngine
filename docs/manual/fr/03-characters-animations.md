# Personnages et animations

## Définir un personnage

Ouvrez le panneau **Personnages** et ajoutez un personnage. Donnez-lui :

- un **nom**, affiché dans la boîte de dialogue (il peut être traduit comme tout autre texte),
- une **couleur**, utilisée pour le nom,
- une ou plusieurs **expressions** : un nom (« content », « fâché ») et l'image à utiliser. Une expression peut aussi être une feuille de sprites animée ou une suite d'images.

Importez d'abord les images dans la **Bibliothèque** (glissez-les dans la fenêtre), puis choisissez-les dans le personnage.

## Mettre un personnage en scène

Ajoutez une étape **Afficher** : choisissez le personnage, l'expression et la position (gauche, centre, droite, une position que vous avez définie, ou une position libre n'importe où sur la scène). Vous pouvez aussi régler la taille, retourner l'image horizontalement ou verticalement et choisir une transition comme un fondu enchaîné. Afficher un personnage déjà en scène met à jour son expression et sa position. **Masquer** le retire.

La **vue de la scène** permet de placer les personnages en les faisant glisser directement sur l'image.

## Animations

Les personnages et autres éléments de la scène peuvent bouger grâce aux interpolations (changements progressifs de position, de taille ou d'opacité) et aux timelines. Le rythme et le style sont réglés par des **courbes** : voir le chapitre Courbes.

## Fonds en parallaxe

Une étape **Fond en parallaxe** remplace le fond par une pile de calques d'images, de l'arrière vers l'avant. Chaque calque a une **profondeur** :

- `1` bouge exactement comme la scène (un fond ordinaire),
- `0` est collé à l'écran (un ciel qui ne bouge jamais),
- entre les deux, on obtient la parallaxe : plus la profondeur est proche de 1, plus le calque suit la caméra,
- au-dessus de `1`, c'est un avant-plan qui bouge plus vite que la scène.

Le déplacement et le zoom de la caméra le pilotent : ajoutez un effet **Zoom**, ou une **Chronologie** avec une piste *déplacement* sur la *caméra*, et chaque calque bouge selon sa propre profondeur. Un déplacement lent et discret (2 à 3 % de l'écran sur 30 secondes) donne déjà beaucoup de profondeur. Utilisez des images avec des zones transparentes pour les calques de devant, sinon ils cachent ceux de derrière. Cochez **Répéter horizontalement** pour un calque qui se répète (nuages, rue) afin qu'il ne montre jamais de bord quand il se déplace. Les calques sont dessinés un peu plus grands que l'écran, pour qu'un léger déplacement ne révèle jamais leurs bords. L'inspecteur montre un petit panorama avec un curseur pour voir la parallaxe sans lancer le jeu. Une étape **Fond** ultérieure remplace les calques.

## Particules

Une étape **Particules** démarre un émetteur : pluie, neige, pétales, poussière ou étincelles, ou le vôtre. Choisissez un **préréglage** (il remplit tous les réglages), puis ajustez : naissances par seconde, durée de vie, zone de naissance, direction et dispersion, vitesse, gravité, balancement, taille, longueur (pour les traînées de pluie), couleurs de la naissance à la mort, opacité, une image facultative, et si les particules passent derrière ou devant les personnages. Un aperçu en direct dans l'inspecteur montre le résultat pendant que vous réglez. Donnez un **nom** à l'émetteur : une étape **Arrêter les particules** portant ce nom (ou vide, pour tous) empêche de nouvelles naissances, et celles qui sont déjà en l'air finissent leur chute ; une nouvelle étape Particules du même nom remplace la première. Les particules s'arrêtent aussi quand l'histoire change de scène.

Les particules sont calculées à partir de l'instant où l'étape s'est exécutée, jamais enregistrées : revenir en arrière, charger une sauvegarde ou passer donnent toujours la même image. L'éditeur prévient quand un émetteur aurait besoin de plus de 1500 particules en même temps (baissez la cadence ou la durée de vie), et l'écran n'en dessine jamais plus de 4000 en tout. Elles sont dessinées sur l'écran, pas dans la caméra : la météo ne tremble donc pas avec la caméra. Avec la **réduction des animations** (voir Accessibilité), aucune particule n'est dessinée.

## Animation par squelette (rigs)

Un **rig** donne vie à un dessin avec des os : respiration, bras qui salue, tête qui tourne. La vue du rig affiche une barre d'étapes (**1. Importer l'image → 2. Dessiner les os → 3. Lier → 4. Poser / animer**) qui indique quoi faire ensuite :

1. **Importer l'image.** Choisissez **Outils > Nouveau rig depuis une image…** (ou le grand bouton de la vue du rig vide, ou déposez-y un PNG). L'image est copiée dans le projet (`assets/images/`) si besoin et devient un rig avec un os `root` aux pieds du personnage et un maillage tracé d'après sa transparence. Elle est cadrée et affichée tout de suite. Les morceaux séparés (un bras, la tête…) s'ajoutent avec **Ajouter une partie (image)…** ou en déposant d'autres PNG sur la vue : chacun devient un emplacement, aligné sur la première image s'il a la même taille.
2. **Dessiner les os** avec l'outil os (**B**) : glissez sur l'image d'une articulation à la suivante (hanches vers cou, épaule vers coude, coude vers main). Un nouvel os pend de l'os sélectionné ; cliquez dans le vide pour commencer une nouvelle chaîne.
3. **Lier.** Cliquez sur **Lier aux os** : chaque image reçoit des poids automatiques (chaque partie suit l'os le plus proche, adouci entre deux os). Pour affiner, prenez le pinceau de poids (**P**) : la carte de chaleur montre l'influence de l'os sélectionné (Ctrl-clic choisit un os), peignez en Ajouter / Retirer / Lisser.
4. **Poser et animer.** Après la liaison, la vue passe en **mode pose** (**Tab**) : sélectionnez un os et tirez son anneau pour plier l'image. Le mode pose ne modifie jamais le rig ; enregistrez des clips dans les panneaux Clips et Timeline.

Si vous ajoutez des os ensuite, la barre d'étapes demande de lier à nouveau. Cliquez enfin sur **Ajouter au projet** : le fichier est enregistré en `assets/rigs/<nom>.rig.ron` et apparaît dans la Bibliothèque. Les images dessinées par un rig doivent être des fichiers du projet.

Pour l'utiliser, ouvrez le personnage, choisissez le type **Rig** pour une expression, puis le rig, la **skin** (tenue), le **clip** et ce qui se passe à la fin (boucle, une fois, aller-retour). Une petite vignette animée montre le résultat. L'image de l'expression reste le **poster** : elle sert aux miniatures et s'affiche pendant le chargement du rig ou s'il est inutilisable (le panneau Problèmes explique pourquoi : fichier absent, clip ou skin inconnus, image absente). Le rig est dessiné à l'échelle du poster, son origine au bas-centre du personnage.

- Montrer le même personnage avec une autre expression **du même rig** fait un fondu croisé des deux clips sur 0,2 seconde au lieu d'un changement sec.
- Les clips continuent pendant qu'une réplique s'affiche ou attend un clic.
- Avec **Réduire les animations**, le rig garde la première pose de son clip.
- Revenir en arrière ou charger une sauvegarde relance le clip : rien n'est sauvegardé à son sujet.

Un assistant IA peut lister et lire les rigs, vérifier une pose sans dessiner, générer un maillage depuis une image et affecter un rig à une expression (voir le guide MCP).

**Construire et animer un rig par MCP.** Un assistant peut faire tout le parcours sans l'interface de l'éditeur : `rig_create` (depuis une image du projet, importée comme asset si besoin : os `root` aux pieds, un slot et un maillage tracé depuis l'alpha ; ou un rig vide), `rig_add_image_part`, `rig_bone_add` / `rig_bone_update` / `rig_bone_reparent` / `rig_bone_delete` (positions relatives au parent ou dans l'espace du rig, ou une `head` et une `tip` comme le cliquer-glisser de l'outil os), `rig_bind` (auto-poids de tous les maillages), `rig_slot_update` (image affichée, teinte, ordre de dessin), `rig_skin_add` / `rig_skin_duplicate` / `rig_skin_delete`, `rig_ik_set` / `rig_ik_remove`, `rig_clip_create` / `rig_clip_update` / `rig_clip_delete`, `rig_clip_key_set` / `rig_clip_key_delete` (rotation, translation et échelle d'os, image et teinte de slot, déformation, ordre de dessin ; chaque clé a une interpolation : préréglage, bézier ou courbe nommée de l'histoire), `rig_add_to_project` et `rig_delete`. Chaque modification est validée (une erreur la refuse et rien n'est écrit ; les avertissements du budget web reviennent avec la réponse) et atomique. Quand le rig est ouvert dans l'éditeur de rig, le changement passe par son document : la vue se rafraîchit et **Ctrl+Z l'annule** ; sinon le fichier est réécrit. `rig_sample` montre ensuite le résultat, y compris de combien le maillage a bougé.


## Voix

Chaque réplique peut avoir une **voix** : sélectionner la réplique et, dans l'inspecteur, choisir un son du projet sous **Voix** (ou *Ajouter un fichier* pour importer un .ogg ou .wav, *Retirer* pour revenir au silence).

* La voix démarre avec la réplique et s'arrête quand le joueur avance (clic, saut, retour arrière, choix). Elle passe par son propre volume **Voix** (Préférences). Une réplique n'est pas rejouée en voix après un retour arrière ou un chargement : le bouton **Voix** du menu rapide (visible quand la réplique à l'écran a une voix) ou le bouton **Voix** de sa ligne dans l'**Historique** la fait réentendre.
* **Le mode auto attend la fin de la voix**, puis applique son délai habituel. Le saut ne lance jamais de voix.
* Une traduction peut avoir ses propres voix : pour la langue choisie, la voix de la traduction remplace celle de la réplique (les répliques sans voix traduite gardent la voix source).

### Importer beaucoup de voix d'un coup

*Importer les voix…* (inspecteur d'une réplique) prend un **dossier** de fichiers audio nommés d'après les répliques : `<id>.ogg`, `.wav`, `.mp3` ou `.flac`, où l'id est celui de la réplique (l'id du nœud de dialogue, ou l'id de son texte ; l'éditeur et les outils `orbit` affichent les deux). Les fichiers d'une traduction vont dans un sous-dossier portant le nom de la langue : `voice/fr/<id>.ogg`. *Depuis un CSV…* fait de même sans renommer les fichiers : une ligne par voix, `réplique,fichier[,langue]` (une ligne d'en-tête est ignorée), le fichier étant un chemin dans le projet. L'import entier est une seule étape d'annulation ; les fichiers hors du projet sont copiés sous `assets/voice/`, les fichiers qui ne correspondent à aucune réplique sont listés et ignorés. Les voix d'une langue sont écrites dans son fichier de traduction (la langue doit déjà exister). L'outil MCP `import_voices` fait pareil (avec `dry_run` pour prévisualiser).

Le validateur signale une voix qui n'est pas un son du projet, et l'export refuse un fichier de voix manquant.
