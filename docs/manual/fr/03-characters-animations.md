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

### Maillages, clips et clés

- **Maillage.** Le maillage d'une image est tracé depuis sa transparence. Prenez l'**outil maillage** (**M**) pour retoucher les sommets (Suppr retire le sommet sélectionné), ou **Recréer le maillage** depuis l'image pour recommencer (cela remplace les retouches manuelles) ; un *espacement intérieur* plus petit donne plus de sommets intérieurs, qui plient mieux. **Suivre seulement l'os du slot** retire les poids pour que le maillage suive son os d'un bloc.
- **Clips.** Dans le panneau des clips de rig, créez un clip (nom, durée, boucle). Sélectionnez-le pour modifier ses pistes et ses clés dans la chronologie : magnétisme sur la grille, la tête de lecture et les autres clés (maintenez **Alt** pour l'ignorer), Suppr retire les clés sélectionnées.
- **Enregistrer des clés.** En mode pose, posez un os et enregistrez : avec l'**auto-clé** activée (le bouton d'enregistrement), chaque changement de pose écrit des clés à la tête de lecture ; ou appuyez sur **K** (*Clé de pose*) pour enregistrer d'un coup tous les os posés à la tête de lecture.
- **Personnages en scène.** Un clip se joue sur un personnage dès qu'une expression utilise le rig (voir la fin de cette section).

### Découper une image en calques

Un personnage dessiné d'un seul bloc peut être séparé en calques posés sur des plans différents et animés par leurs propres os — typiquement, pour un visage d'anime : **mèches de devant / paupières / visage / cheveux de derrière**. Sélectionnez la partie dans la liste des slots (Hiérarchie du rig) et prenez l'**outil découpe** (ciseaux, en mode repos) :

- **Lasso** : tracez autour des pixels. **Polygone** : cliquez les coins, puis recliquez le premier point, double-cliquez ou appuyez sur **Entrée**. **Baguette magique** : cliquez une couleur (tolérance, contiguë ou partout).
- **Maj** ajoute à la sélection, **Ctrl** en retire, **Échap** l'efface. La sélection apparaît teintée sur l'image.
- Choisissez **Devant** ou **Derrière**, puis **Découper en nouvelle partie** : les pixels sélectionnés deviennent une nouvelle image (`<partie>_cut.png`, à côté de l'originale, déclarée dans le projet) et un nouveau slot avec son propre maillage tracé, **exactement à leur place**, dessiné juste devant (ou derrière) la source et suivant l'**os sélectionné** (dessinez d'abord un os le long d'une mèche, sélectionnez-le, puis découpez : la mèche se balance dessus).
- **Boucher le trou** (coché par défaut) remplit le trou laissé dans la source à partir des pixels autour — convenable sous des mèches, sur la peau ou des aplats. Décochez pour garder l'original dessous. Si le trou change le contour de la source, son maillage est retracé.

Toute la découpe (nouveau slot, nouvelle image, trou bouché) s'annule **en une fois**. Le fichier original n'est jamais écrasé : la première modification d'une image importée écrit une copie (`<nom>_edit.png`) que le rig utilise ensuite.

### Peindre sur une partie

L'**outil peinture** (pinceau large) peint sur l'image de la partie sélectionnée : **pinceau** (taille, dureté, opacité, couleur), **gomme**, **tampon de duplication** (**Alt-clic** fixe la source), **pot de peinture** (tolérance) et **pipette** (ou **Alt-clic** avec le pinceau). **[** et **]** changent la taille. Chaque trait s'annule d'un coup, dans l'historique du rig. Les images sont écrites à l'enregistrement du rig ; cochez **Peindre sur le fichier original** si vous voulez vraiment modifier le fichier importé lui-même.

**Une image partagée n'est jamais modifiée par surprise.** Avant la première modification de pixels d'une image (peinture, gomme, tampon, pot de peinture, découpe qui bouche le trou, ou **Modifier dans une autre application**), l'éditeur cherche tout ce qui affiche le même fichier : les autres rigs (toutes leurs skins), les autres slots du même rig, les expressions de personnages, les fonds et l'interface. S'il y en a, une fenêtre les liste et propose **Faire une copie pour ce rig** (par défaut : un nouveau `<rig>_<slot>.png` est écrit, déclaré comme asset et le slot bascule dessus, dans la même étape d'annulation que votre trait) ou **Modifier l'image partagée** (tous les utilisateurs afficheront votre changement). Le choix est retenu pour cette image jusqu'à la fermeture de l'éditeur. L'inspecteur du slot affiche un badge **Partagée par N** sur une telle image (survolez-le pour la liste) avec un bouton **Rendre unique** qui fait la copie tout de suite. **Dupliquer le rig…** copie les images par défaut : un personnage dupliqué ne repeint donc pas son original. MCP : `rig_cut` remplit une copie nommée `<rig>_<slot>.png` pour une image partagée (`copy_if_shared`, vrai par défaut) ; avec `copy_if_shared: false` il refuse sauf si `allow_shared: true`.

**En mode pose aussi.** Pinceau, gomme, tampon, pot de peinture, pipette et les sélections de découpe (lasso, polygone, baguette magique) fonctionnent aussi sur le rig **posé** — une pose, un clip à la tête de lecture, l'aperçu d'un fondu. Vous peignez là où la partie est affichée ; l'éditeur retrouve le pixel correspondant de l'image à travers les os et la déformation du maillage, donc la modification va dans l'image elle-même et se voit dans toutes les poses. Le pinceau garde sa taille à l'écran (sur une partie étirée il couvre moins de pixels de l'image). La pose n'est jamais modifiée et aucune clé n'est enregistrée. Limites : là où un maillage se replie sur lui-même, c'est la couche dessinée au-dessus qui est peinte ; un contour de sélection qui traverse un pli ou une zone très étirée est converti point par point et peut être déformé — découpez ces formes en pose de repos. Si un clip affiche une autre image pour la partie que la pose de repos, **Découper en nouvelle partie** vous demande de revenir à la pose de repos.

**Nouveau calque…** crée une image vide (par défaut à la taille de la partie sélectionnée, alignée dessus) comme nouvelle partie juste devant — la façon de **peindre des paupières fermées** sur des yeux ouverts. **Retracer le maillage** retrace le maillage de la partie après avoir gommé ou peint hors de son contour.

**Modifier dans une app externe** ouvre l'image dans votre logiciel de dessin (celui du système, ou le programme choisi dans le menu **⋯** à côté). À chaque enregistrement là-bas, la partie se recharge dans l'éditeur de rig (une annulation ; maillage retracé si le contour a changé).

### Ordre d'affichage

La liste des slots montre **en haut** ce qui est dessiné par-dessus, avec le numéro d'ordre de chaque slot (**1** est dessiné en premier, derrière tout). **Glissez un slot** dans la liste pour le déplacer, ou utilisez son menu clic droit. Ordre typique d'un visage, de l'arrière vers l'avant : cheveux de derrière, visage, yeux, paupières, mèches de devant.

### Faire cligner des yeux

1. Peignez les paupières fermées sur un **Nouveau calque…** au-dessus du visage, ou découpez-les d'une image « yeux fermés » collée par-dessus. Gardez ce calque **sous les mèches de devant** dans l'ordre d'affichage, pour que les mèches qui traversent les yeux restent visibles.
2. Dans le panneau Clips, créez un clip (**Nouveau clip**, par exemple `idle`, 3 secondes, **Sans fin**).
3. Sélectionnez le slot des paupières, ajoutez une piste **Pièce jointe** : une clé à **0 s** sur **(aucune)** (masqué), une clé à **2,85 s** sur l'image des paupières, une clé à **2,95 s** de nouveau sur **(aucune)**. Les clés de pièce jointe basculent d'un coup (sans intermédiaire) : c'est exactement un clignement.
4. Lancez le clip : les yeux se ferment un dixième de seconde toutes les trois secondes. Un autre clip peut cligner deux fois, ou à un autre rythme.

Par MCP, ce sont des `rig_clip_key_set` avec `track: "slot_attachment"` et pour `value` l'id de la pièce jointe ou `null`.

Pour l'utiliser, ouvrez le personnage, choisissez le type **Rig** pour une expression, puis le rig, la **skin** (tenue), le **clip** et ce qui se passe à la fin (boucle, une fois, aller-retour). Une petite vignette animée montre le résultat. L'image de l'expression reste le **poster** : elle sert aux miniatures et s'affiche pendant le chargement du rig ou s'il est inutilisable (le panneau Problèmes explique pourquoi : fichier absent, clip ou skin inconnus, image absente). Le rig est dessiné à l'échelle du poster, son origine au bas-centre du personnage.

- Montrer le même personnage avec une autre expression **du même rig** fait un fondu croisé des deux clips sur 0,2 seconde au lieu d'un changement sec.
- Les clips continuent pendant qu'une réplique s'affiche ou attend un clic.
- Avec **Réduire les animations**, le rig garde la première pose de son clip.
- Revenir en arrière ou charger une sauvegarde relance le clip : rien n'est sauvegardé à son sujet.

Un assistant IA peut lister et lire les rigs, vérifier une pose sans dessiner, générer un maillage depuis une image et affecter un rig à une expression (voir le guide MCP).

**Construire et animer un rig par MCP.** Un assistant peut faire tout le parcours sans l'interface de l'éditeur : `rig_create` (depuis une image du projet, importée comme asset si besoin : os `root` aux pieds, un slot et un maillage tracé depuis l'alpha ; ou un rig vide), `rig_add_image_part`, `rig_bone_add` / `rig_bone_update` / `rig_bone_reparent` / `rig_bone_delete` (positions relatives au parent ou dans l'espace du rig, ou une `head` et une `tip` comme le cliquer-glisser de l'outil os), `rig_bind` (auto-poids de tous les maillages), `rig_slot_update` (image affichée, teinte, ordre de dessin), `rig_skin_add` / `rig_skin_duplicate` / `rig_skin_delete`, `rig_ik_set` / `rig_ik_remove`, `rig_clip_create` / `rig_clip_update` / `rig_clip_delete`, `rig_clip_key_set` / `rig_clip_key_delete` (rotation, translation et échelle d'os, image et teinte de slot, déformation, ordre de dessin ; chaque clé a une interpolation : préréglage, bézier ou courbe nommée de l'histoire), `rig_add_to_project`, `rig_delete` et `rig_cut` (découpe les pixels d'un polygone, donné en coordonnées pixels de l'image, en une nouvelle partie au même endroit ; la source bouchée est écrite comme nouvelle image). Chaque modification est validée (une erreur la refuse et rien n'est écrit ; les avertissements du budget web reviennent avec la réponse) et atomique. Quand le rig est ouvert dans l'éditeur de rig, le changement passe par son document : la vue se rafraîchit et **Ctrl+Z l'annule** ; sinon le fichier est réécrit. `rig_sample` montre ensuite le résultat, y compris de combien le maillage a bougé.


### Importer un fichier à calques (PSD, Krita, OpenRaster)

Si vous dessinez votre personnage dans Photoshop, Krita ou un éditeur OpenRaster (un calque par partie : tête, bras, yeux...), inutile d'exporter chaque calque à la main. Dans l'éditeur de rig, **Nouveau rig depuis un fichier à calques…** (ou **Importer un fichier à calques…** pour ajouter au rig ouvert) prend un fichier `.psd`, `.kra` ou `.ora`. Chaque calque raster visible devient une image PNG de votre projet (rognée à ses pixels, dans `images/<nom du fichier>/`), un slot nommé d'après le calque et un maillage placé à sa position exacte sur le canevas, dans l'ordre de la pile de calques. L'origine du rig est le centre bas du canevas, comme pour un rig fait depuis une seule image. L'opacité du calque devient la teinte du slot.

- **Groupes.** L'éditeur demande s'il faut créer un os par groupe de calques (imbriqués comme les groupes) ; les calques d'un groupe suivent alors son os. Répondez non et tout suit l'os `root`.
- **Calques masqués** : importés aussi, comme slots sans image active : idéal pour des variantes comme « yeux fermés » ou « bouche ouverte ». Affichez-en une avec une clé d'image dans un clip, ou en choisissant son image dans l'inspecteur.
- **Ce qui n'est pas lu** (le fichier s'importe quand même, avec un avertissement) : calques de texte, de réglage, d'objet dynamique et de forme, masques de calque, modes de fusion, données PSD compressées en ZIP. Seuls les fichiers RVB à 8 ou 16 bits par canal sont lus ; convertissez d'abord les fichiers CMJN ou en niveaux de gris.
- L'import est une seule étape d'annulation. Les maillages ne sont pas pondérés : dessinez les os, puis **Lier**.
- La fonction vit dans l'éditeur et le serveur MCP seulement : les jeux exportés ne la contiennent pas et ne grossissent pas.

Par MCP, `rig_import_layers` fait la même chose (`source`, `rig` optionnel pour ajouter à un rig existant, `groups_as_bones`, `include_hidden`) et renvoie les avertissements dans `import_warnings`.

### Dupliquer un rig, copier des clips, recentrer

Trois outils de l'éditeur de rig (le menu clé à molette de la barre d'outils, ou le menu **Outils**) font gagner beaucoup de temps quand on fabrique plusieurs personnages à partir d'un même squelette. Chacun est une seule étape d'annulation, vit dans l'éditeur et le serveur MCP uniquement, et n'ajoute rien aux jeux exportés.

- **Dupliquer le rig…** copie le rig ouvert sous un nouveau nom dans `assets/rigs/`, le déclare comme asset du projet et l'ouvre. Cochez **Copier aussi les images** pour donner à la copie ses propres images dans `images/<nom>/` ; décoché, les deux rigs partagent les mêmes images (repeindre l'une change l'autre). Un fichier existant n'est jamais écrasé. MCP : `rig_duplicate`.
- **Copier des clips d'un autre rig…** amène des clips d'un autre rig dans celui-ci. Choisissez le rig source (un du projet, ou **Parcourir…**) et cochez les clips. Les pistes sont associées aux os et aux slots **par nom** ; **Associer par position dans l'arbre des os** sert aux rigs dont les os ont des noms différents (le même rang d'enfant depuis la racine ; les slots suivent leur os). **Adapter les translations aux longueurs d'os** multiplie les clés de translation par le rapport des longueurs des os associés (désactivé par défaut). Les clés d'image de slot suivent l'identifiant ou le nom de fichier de l'image. Les clés de déformation (décalage de sommets) sont abandonnées sauf si les deux maillages ont exactement le même nombre de sommets. Un clip dont le nom existe déjà est copié avec un numéro (`walk2`) : rien n'est écrasé. Quand quelque chose n'a pas pu être associé (os, slots, images, pistes de déformation), un rapport le liste. MCP : `rig_copy_clips`.
- **Recentrer sur l'origine…** corrige les personnages qui atterrissent au mauvais endroit sur la scène. L'origine du rig `(0, 0)` est l'endroit où le bas-centre du personnage est posé ; l'éditeur la montre maintenant dans la vue comme une cible (ambre quand elle est loin du dessin). L'outil mesure les vraies limites des images visibles en pose de repos (le skin affiché, ou tous les skins), puis décale les os racine et les maillages pour que le bas-centre du dessin tombe exactement sur l'origine. Les clips restent valides car ils sont relatifs à la pose de repos. MCP : `rig_recenter`.

Quand l'origine est loin du dessin visible (à plus de 25 % de sa largeur sur le côté, ou à plus de 10 % de sa hauteur du bord bas), un avertissement apparaît dans la liste **Problèmes** du rig et dans le panneau Problèmes du projet.

Dans la liste des slots, un slot grisé avec une icône d'œil barré ne dessine rien dans le skin affiché (aucune image active) : survolez-le pour lire pourquoi. Choisissez son image dans l'inspecteur, ou changez de skin.

## Voix

Chaque réplique peut avoir une **voix** : sélectionner la réplique et, dans l'inspecteur, choisir un son du projet sous **Voix** (ou *Ajouter un fichier* pour importer un .ogg ou .wav, *Retirer* pour revenir au silence).

* La voix démarre avec la réplique et s'arrête quand le joueur avance (clic, saut, retour arrière, choix). Elle passe par son propre volume **Voix** (Préférences). Une réplique n'est pas rejouée en voix après un retour arrière ou un chargement : le bouton **Voix** du menu rapide (visible quand la réplique à l'écran a une voix) ou le bouton **Voix** de sa ligne dans l'**Historique** la fait réentendre.
* **Le mode auto attend la fin de la voix**, puis applique son délai habituel. Le saut ne lance jamais de voix.
* Une traduction peut avoir ses propres voix : pour la langue choisie, la voix de la traduction remplace celle de la réplique (les répliques sans voix traduite gardent la voix source).

### Importer beaucoup de voix d'un coup

*Importer les voix…* (inspecteur d'une réplique) prend un **dossier** de fichiers audio nommés d'après les répliques : `<id>.ogg`, `.wav`, `.mp3` ou `.flac`, où l'id est celui de la réplique (l'id du nœud de dialogue, ou l'id de son texte ; l'éditeur et les outils `orbit` affichent les deux). Les fichiers d'une traduction vont dans un sous-dossier portant le nom de la langue : `voice/fr/<id>.ogg`. *Depuis un CSV…* fait de même sans renommer les fichiers : une ligne par voix, `réplique,fichier[,langue]` (une ligne d'en-tête est ignorée), le fichier étant un chemin dans le projet. L'import entier est une seule étape d'annulation ; les fichiers hors du projet sont copiés sous `assets/voice/`, les fichiers qui ne correspondent à aucune réplique sont listés et ignorés. Les voix d'une langue sont écrites dans son fichier de traduction (la langue doit déjà exister). L'outil MCP `import_voices` fait pareil (avec `dry_run` pour prévisualiser).

Le validateur signale une voix qui n'est pas un son du projet, et l'export refuse un fichier de voix manquant.
