# Scènes 3D

> Expérimental. Les fonctions 3D sont masquées par défaut et peuvent changer d'une version à l'autre. Activez-les avec **Affichage > Options > Fonctionnalités 3D expérimentales** (ou lancez l'éditeur avec la variable d'environnement `ORBIT_EXPERIMENTAL_3D=1`). Ce chapitre du manuel n'apparaît que lorsqu'elles sont activées. Les projets qui utilisent déjà la 3D continuent de se charger et de se jouer interrupteur éteint.

Un visual novel peut utiliser une scène 3D comme décor : une pièce où l'on peut regarder autour, une rue avec une caméra mobile. Les personnages 2D et la boîte de dialogue sont dessinés par-dessus.

## Construire une scène 3D

Ouvrez l'**éditeur de scènes 3D** (il ouvre trois panneaux) :

- **Vue 3D** : la scène telle que la voit la caméra. Le glisser avec le bouton droit tourne autour, le bouton du milieu déplace, la molette zoome.
- **Hiérarchie 3D** : la liste des objets de la scène (modèles, lumières, caméras).
- **Inspecteur 3D** : les réglages de l'objet sélectionné (position, rotation, taille, couleur, intensité de la lumière).

Créez une nouvelle scène, importez des modèles 3D, placez des lumières et ajoutez une ou plusieurs caméras nommées. Une nouvelle scène démarre avec un soleil et une caméra nommée `main`. Enregistrez la scène (c'est un fichier `.scene3d.ron`) ; les modèles importés sont rangés dans le dossier `models` à côté d'elle, enregistrez donc la scène avant d'importer.

### Ce que vous pouvez ajouter

Depuis le menu **Ajouter** de la hiérarchie 3D (ou **Ajouter un enfant** au clic droit) :

- **Forme** : cube, sphère, plan, cylindre, cône, capsule ou tore, fabriqués par le moteur. Les formes n'ont besoin d'aucun fichier. Elles servent pour un sol, des murs, des meubles provisoires ou des accessoires simples. Une nouvelle forme est posée sur le sol et reçoit le matériau **Default** de la scène (blanc), que vous pouvez modifier ou remplacer dans l'inspecteur. L'inspecteur règle aussi sa taille, son rayon, sa hauteur et son nombre de segments (plus de segments, plus c'est rond).
- **Lumières** (directionnelle, ponctuelle, projecteur), **caméras** et **nœuds vides** (pour grouper des objets).
- **Modèles** avec **Importer un glTF** : fichiers `.glb`, ou fichiers `.gltf` accompagnés de leurs fichiers `.bin` et de leurs textures, comme les exports de Sketchfab. Le modèle et ses fichiers sont copiés dans le dossier `models` à côté de la scène. Les matériaux de l'ancien modèle « specular-glossiness » sont convertis, leurs couleurs restent proches de l'original. Un modèle garde son **squelette, ses poids et ses animations** : voir *Poser un personnage* plus bas. Si un import échoue, un message en donne la raison (par exemple un fichier `.bin` manquant).

Vous pouvez aussi déposer des fichiers `.glb` / `.gltf` sur la vue, ou glisser un modèle de la liste **Modèles** de la hiérarchie dans la vue 3D pour le placer.

### Ciel et textures

Quand rien n'est sélectionné, l'inspecteur 3D montre l'**environnement** : choisissez une couleur unie ou une **skybox** (n'importe quelle image de la scène, voir plus bas), la lumière ambiante et le brouillard.

Un matériau a quatre emplacements de texture (couleur de base, occlusion / rugosité / métal, normale, émission). Chacun propose les textures de la scène : celles qui viennent de vos modèles importés, et les images que vous ajoutez avec **Importer…** (PNG ou JPEG). Les images importées sont rangées dans le dossier `models` à côté du fichier de la scène : enregistrez d'abord la scène. Pour une skybox, prenez un panorama large (image équirectangulaire) ou une cubemap.

## Outils de la vue

- **Déplacer / Tourner / Mettre à l'échelle** avec **W**, **E**, **R** (le manipulateur est sur la sélection). **F** cadre la sélection. **Suppr** la supprime et **Ctrl+D** la duplique.
- **Magnétisme** (barre d'outils ; maintenez **Ctrl** pour l'inverser) : 0,25 m pour les déplacements, 15 degrés pour les rotations, pas de 0,1 pour l'échelle.
- **Vues prédéfinies** : **1** face, **3** droite, **7** dessus ; avec **Ctrl**, le côté opposé (arrière, gauche, dessous). La barre d'outils a un menu **Point de vue** avec les six vues.
- **G** pose la sélection sur le sol.
- La vue dessine le **frustum de chaque caméra** et un **repère pour chaque lumière**, pour voir ce qui pointe où.
- **Afficher ce que voit la caméra du jeu** ajoute un petit aperçu en incrustation de la caméra du jeu (la caméra sélectionnée, sinon la première). **Regarder par cette caméra** (inspecteur de caméra) voit la scène depuis elle.
- **Aligner sur la vue** place la caméra du jeu (celle sélectionnée, sinon la première) sur la vue actuelle : tournez jusqu'à un bel angle, puis alignez.

## Poser un personnage

Un personnage glTF qui a un squelette montre une section **Squelette** dans l'inspecteur, avec son nombre d'os.

- Cliquez sur **Pose** (ou appuyez sur **P**) pour entrer en **mode pose**. L'arbre des os liste les os. Cliquez un os dans la vue ou dans l'arbre, puis tirez les anneaux du manipulateur pour le tourner (sur l'os racine, **W** le déplace). **Suppr** réinitialise l'os sélectionné, **Réinitialiser la pose** les réinitialise tous, **Échap** quitte le mode pose. Chaque modification s'annule.
- **Poses enregistrées** : saisissez un nom et **Enregistrer la pose** ; **Appliquer** rappelle une pose enregistrée.
- **Animations** : les animations livrées avec le fichier glTF sont listées. **Lire**, **Pause** et **Arrêter l'aperçu** en jouent une dans la vue, et vous pouvez la parcourir. **Garder cette image comme pose** transforme la pose affichée à cet instant en pose du personnage dans la scène. **Nouvelle animation** crée une animation image par image, et **Clé de pose ici** y enregistre la pose actuelle comme clé à cet instant.

## Ajouter la scène au projet

Une scène 3D n'est utilisable dans l'histoire qu'une fois qu'elle fait partie du projet. Cliquez sur **Ajouter au projet** (l'icône de dossier de la barre d'outils de la vue) : la scène est enregistrée sous `assets/3d/<nom>.scene3d.ron` et apparaît dans la bibliothèque d'assets, catégorie « Scène 3D ». Les modèles sont copiés avec elle. Vous pouvez aussi déposer un fichier `.scene3d.ron` existant sur la bibliothèque. Les enregistrements suivants mettent à jour le même asset, et une scène 3D importée dans la bibliothèque garde son nom et son dossier `models`.

Le marqueur de modifications non enregistrées du projet compte aussi la scène 3D ouverte : enregistrer le projet l'enregistre.

## L'utiliser dans l'histoire

Trois étapes pilotent la 3D dans l'histoire. Elles sont dans le sous-menu **3D** du menu d'insertion, à côté des étapes de décor et de personnage :

- **Afficher une scène 3D** : affiche une scène 3D comme décor, avec une caméra et une transition facultative. Choisissez la scène dans l'inspecteur, puis la caméra dans la liste (les caméras sont lues dans le fichier de la scène ; « première caméra de la scène » est le choix par défaut).
- **Caméra 3D** : passe à une autre caméra nommée de la scène affichée, par exemple pour couper ou aller sur un gros plan.
- **Masquer la scène 3D** : revient à un décor 2D normal.

Passer de la 2D à la 3D et inversement utilise les mêmes transitions que n'importe quel décor. Dans l'Éditeur de vue et la Preview, une scène qui affiche une scène 3D dessine ses modèles, pas seulement le ciel.

**Afficher dans l'histoire.** Dans la vue 3D, **Afficher la scène 3D dans l'histoire** ajoute la scène au projet si besoin, sélectionne l'étape qui l'affiche (ou en insère une après la ligne sélectionnée) et lance la preview à partir de là. Il faut une scène dans l'histoire où insérer.

**Bandeau de copie périmée.** L'histoire affiche le fichier qui est dans le projet. Si la scène que vous modifiez est un autre fichier, un bandeau indique que « l'histoire affiche une copie de cette scène » et que vos modifications ne l'atteignent pas. Cliquez sur **Utiliser le fichier de l'histoire** : cela enregistre votre scène sur le fichier de l'histoire, copie ses modèles à côté et modifie ce fichier à partir de maintenant.

Si une étape nomme une caméra que la scène n'a pas (par exemple après l'avoir renommée), si un fichier de scène est invalide, ou si la scène place un modèle absent de son dossier `models`, le panneau **Problèmes** le signale.

## Dans le jeu exporté

À l'export, le jeu emporte les modèles, textures et formes que ses scènes 3D utilisent. Les modèles et images du dossier `models` qu'aucune scène n'utilise sont laissés de côté. Si une scène désigne un modèle introuvable, le journal d'export le signale et la scène s'affiche sans lui.

Le réglage **Qualité 3D (ordinateur)** du profil d'export (Basse ou Haute) décide de la qualité de rendu des scènes 3D dans les jeux Windows et Linux ; les jeux web et Android utilisent toujours le profil web. Le panneau Export mesure vos scènes (lumières, ombres, triangles, textures) et dit si elles tiennent dans le profil ; au-delà, le jeu fonctionne quand même, avec les lumières et ombres en trop abandonnées.

Un jeu avec des modèles ou des formes 3D demande un player de cette version d'OrbitEngine ou d'une version plus récente. Un player plus ancien refuse le jeu avec un message au lieu d'afficher des scènes vides. Les jeux 2D ne sont pas concernés.

## Travailler avec un assistant IA

L'assistant (MCP) peut lire et modifier vos scènes 3D : les lister, lire leurs nœuds, lumières, caméras et matériaux, ajouter ou déplacer des objets (formes comprises), changer le ciel, créer une nouvelle scène, et poser le squelette d'un personnage (rotations des os, enregistrement, application et suppression de poses). Chaque modification est vérifiée, et refusée en bloc si elle casserait la scène. Avec l'éditeur ouvert et une carte graphique, l'assistant peut aussi prendre une image d'une scène depuis n'importe laquelle de ses caméras pour vérifier son travail. Ces modifications réécrivent le fichier de la scène ; elles ne sont pas dans l'historique d'annulation : fermez et rouvrez la scène dans l'éditeur 3D pour les voir.

## Conseils

- Commencez avec une lumière et une caméra, puis ajoutez des détails.
- Testez dans le panneau Preview : il utilise le même moteur de rendu que le jeu exporté.
- Si un modèle ne s'affiche pas, vérifiez que son fichier est dans le dossier `models` à côté de la scène et lisez le panneau Problèmes.
