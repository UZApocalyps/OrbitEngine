# Scènes 3D

Un visual novel peut utiliser une scène 3D comme décor : une pièce où l'on peut regarder autour, une rue avec une caméra mobile. Les personnages 2D et la boîte de dialogue sont dessinés par-dessus.

## Construire une scène 3D

Ouvrez l'**éditeur de scène 3D**. Il comporte trois panneaux :

- **Vue 3D** : la scène telle que la voit la caméra. Tournez, déplacez et zoomez pour regarder autour.
- **Hiérarchie 3D** : la liste des objets de la scène (modèles, lumières, caméras).
- **Inspecteur 3D** : les réglages de l'objet sélectionné (position, rotation, taille, couleur, intensité de la lumière).

Créez une nouvelle scène, importez des modèles 3D, placez des lumières et ajoutez une ou plusieurs caméras nommées. Une nouvelle scène démarre avec un soleil et une caméra nommée `main`.

### Ce que vous pouvez ajouter

Depuis le menu **Ajouter** de la hiérarchie 3D (ou **Ajouter un enfant** au clic droit) :

- **Forme** : cube, sphère, plan, cylindre, cône, capsule ou tore, fabriqués par le moteur. Les formes n'ont besoin d'aucun fichier. Elles servent pour un sol, des murs, des meubles provisoires ou des accessoires simples. Une nouvelle forme est posée sur le sol et reçoit le matériau **Default** de la scène (blanc), que vous pouvez modifier ou remplacer dans l'inspecteur. L'inspecteur règle aussi sa taille, son rayon, sa hauteur et son nombre de segments (plus de segments, plus c'est rond).
- **Lumières** (directionnelle, ponctuelle, projecteur), **caméras** et **nœuds vides** (pour grouper des objets).
- **Modèles** avec **Importer un glTF** : fichiers `.glb`, ou fichiers `.gltf` accompagnés de leurs fichiers `.bin` et de leurs textures, comme les exports de Sketchfab. Le modèle et ses fichiers sont copiés dans le dossier `models` à côté de la scène. Les matériaux de l'ancien modèle « specular-glossiness » sont convertis, leurs couleurs restent proches de l'original. Les animations et les squelettes ne sont pas importés : le modèle s'affiche dans sa pose de repos. Si un import échoue, un message en donne la raison (par exemple un fichier `.bin` manquant).

### Ciel et textures

Quand rien n'est sélectionné, l'inspecteur 3D montre l'**environnement** : choisissez une couleur unie ou une **skybox** (n'importe quelle image de la scène, voir plus bas), la lumière ambiante et le brouillard.

Un matériau a quatre emplacements de texture (couleur de base, occlusion / rugosité / métal, normale, émission). Chacun propose les textures de la scène : celles qui viennent de vos modèles importés, et les images que vous ajoutez avec **Importer…** (PNG ou JPEG). Les images importées sont rangées dans le dossier `models` à côté du fichier de la scène : enregistrez d'abord la scène. Pour une skybox, prenez un panorama large (image équirectangulaire) ou une cubemap.

### Ajouter la scène au projet

Une scène 3D n'est utilisable dans l'histoire qu'une fois qu'elle fait partie du projet. Cliquez sur **Ajouter au projet** (l'icône de dossier de la barre d'outils de la vue) : la scène est enregistrée sous `assets/3d/<nom>.scene3d.ron` et apparaît dans la bibliothèque d'assets, catégorie « Scène 3D ». Vous pouvez aussi déposer un fichier `.scene3d.ron` existant sur la bibliothèque. Les enregistrements suivants mettent à jour le même asset.

Le marqueur de modifications non enregistrées du projet compte aussi la scène 3D ouverte : enregistrer le projet l'enregistre.

## L'utiliser dans l'histoire

Trois étapes pilotent la 3D dans l'histoire. Elles sont dans le menu « ajouter une étape », à côté des étapes de décor et de personnage :

- **Afficher une scène 3D** : affiche une scène 3D comme décor, avec une caméra et une transition facultative. Choisissez la scène dans l'inspecteur, puis la caméra dans la liste (les caméras sont lues dans le fichier de la scène ; « première caméra de la scène » est le choix par défaut).
- **Caméra 3D** : passe à une autre caméra nommée de la scène affichée, par exemple pour couper ou aller sur un gros plan.
- **Masquer la scène 3D** : revient à un décor 2D normal.

Passer de la 2D à la 3D et inversement utilise les mêmes transitions que n'importe quel décor.

Si une étape nomme une caméra que la scène n'a pas (par exemple après l'avoir renommée), ou si un fichier de scène est invalide, le panneau **Problèmes** le signale.

## Dans le jeu exporté

À l'export, le jeu emporte les modèles, textures et formes que ses scènes 3D utilisent. Les modèles et images du dossier `models` qu'aucune scène n'utilise sont laissés de côté. Si une scène désigne un modèle introuvable, le journal d'export le signale et la scène s'affiche sans lui.

Un jeu avec des modèles ou des formes 3D demande un player de cette version d'OrbitEngine ou d'une version plus récente. Un player plus ancien refuse le jeu avec un message au lieu d'afficher des scènes vides. Les jeux 2D ne sont pas concernés.

## Travailler avec un assistant IA

L'assistant (MCP) peut lire et modifier vos scènes 3D : les lister, lire leurs nœuds, lumières, caméras et matériaux, ajouter ou déplacer des objets (formes comprises), changer le ciel, créer une nouvelle scène. Chaque modification est vérifiée, et refusée en bloc si elle casserait la scène. Avec l'éditeur ouvert et une carte graphique, l'assistant peut aussi prendre une image d'une scène depuis n'importe laquelle de ses caméras pour vérifier son travail. Ces modifications réécrivent le fichier de la scène ; elles ne sont pas dans l'historique d'annulation : fermez et rouvrez la scène dans l'éditeur 3D pour les voir.

## Conseils

- Commencez avec une lumière et une caméra, puis ajoutez des détails.
- Testez dans le panneau Preview : il utilise le même moteur de rendu que le jeu exporté.
