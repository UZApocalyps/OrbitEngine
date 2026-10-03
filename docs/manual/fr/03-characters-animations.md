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

> L'animation de personnages par squelette n'est pas encore disponible.
