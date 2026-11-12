# Tutoriel : point & click, explorer une maison

Dans une scène point & click, l'histoire s'arrête sur une image et attend que le joueur clique sur une **zone** : une porte, un objet, un personnage. Ce tutoriel construit une petite maison à explorer avec un assistant, puis la rend vraiment vôtre : des portes placées sur l'image réelle, une clé à trouver, une porte fermée qui s'ouvre une fois la clé prise, et un objet dont la zone épouse le dessin.

## Objectif

Une maison de trois pièces (entrée, cuisine, chambre). Des portes relient les pièces, une clé traîne dans l'entrée, et la porte de la chambre reste grisée tant que la clé n'est pas dans votre poche.

## Avant de commencer

- Un projet avec au moins une image de décor. Trois images différentes (une par pièce) sont préférables ; l'assistant propose la première image de la bibliothèque pour chaque pièce et vous pouvez changer chacune.
- Une scène ouverte dans le panneau **Scène**. L'assistant insère la maison à l'étape sélectionnée.
- Facultatif, pour la dernière étape : un PNG d'objet (un vase, une lampe) à fond transparent.

## Étapes

### 1. Lancer l'assistant de carte de navigation

1. Sélectionnez l'étape après laquelle la maison doit commencer (par exemple la dernière réplique de votre introduction).
2. Choisissez **Édition > Insérer > Nouvelle carte de navigation (maison)…**. La fenêtre explique ce qu'elle écrit : un label et une carte point & click par pièce, une zone de porte vers chaque pièce reliée, une zone « Retour » et un exemple de porte fermée à clé.
3. Dans **Pièces**, nommez chaque pièce. L'assistant propose *Entrée*, *Cuisine* et *Chambre* ; utilisez **Ajouter une pièce** pour en avoir plus (douze au maximum) et **Supprimer la pièce** pour en retirer une. Pour chaque pièce, cliquez sur **Choisir un décor**.
4. Dans **Portes entre les pièces**, cochez les paires de pièces reliées : une porte est écrite dans chaque sens. Par défaut, l'entrée est reliée à la cuisine et à la chambre.
5. **Porte fermée à clé (exemple)** propose l'une des portes, ou **Aucune**. Gardez la porte de l'entrée vers la chambre : la clé se trouvera dans l'entrée.
6. Laissez cochée **Une zone « Retour » dans chaque pièce** : chaque pièce sauf la première reçoit une zone qui ramène à la pièce d'où l'on vient.
7. Cliquez sur **Créer la carte**. L'assistant ajoute, en une seule étape d'annulation, un label et une étape **Zones** par pièce, ainsi qu'une variable `cle_maison` (faux au départ) dans le panneau **Variables**.

Si l'assistant signale qu'une pièce n'a aucune porte, reliez-la à une autre pièce : il lui faut au moins une zone.

### 2. Regarder ce qui a été écrit

1. Sélectionnez l'étape **Zones** de la première pièce dans le panneau Scène. L'**Inspecteur** montre sa **Carte** : **Afficher son propre décor** (l'image de la pièce), l'**Apparence (surbrillance au survol)** et la liste des **Zones**.
2. Chaque zone a un **Nom**, une **Info-bulle** affichée au survol (*Vers Cuisine*, *Vers Chambre*, *Une clé*, *Retour*) et **Mène à** : le label de la pièce qu'elle ouvre.
3. Jouez la scène avec **Preview** (F6) : vous naviguez déjà entre les pièces. Pour l'instant, les portes ne sont que des rectangles provisoires.

### 3. Poser les portes sur la vraie image

1. L'étape **Zones** étant sélectionnée, regardez le panneau **Éditeur de vue** : il montre la pièce et dessine ses zones. Une barre **Zones :** propose **Sélection**, **Rectangle**, **Ellipse** et **Polygone**.
2. Avec **Sélection**, cliquez sur une zone pour la choisir. Faites-la glisser sur la porte de l'image ; tirez ses coins pour la redimensionner. Pour une porte qui n'est pas un rectangle, utilisez **Polygone** : cliquez les points un par un, puis cliquez de nouveau sur le premier (ou appuyez sur Entrée) pour fermer la forme ; Retour arrière enlève le dernier point et Échap annule. Double-cliquez sur un bord pour ajouter un point, clic droit sur un point pour l'enlever.
3. Pour ajouter une zone, choisissez **Rectangle**, **Ellipse** ou **Polygone** et dessinez sur l'image. **Supprimer la zone** (ou la touche Suppr) retire la zone sélectionnée. Dessinez-en autant que vous voulez et réglez leur **Nom**, **Info-bulle** et **Mène à** dans l'Inspecteur.
4. Faites de même pour les portes des autres pièces.

### 4. Faire fonctionner la clé

L'assistant a écrit la zone de la clé pour vous. Sélectionnez la zone nommée `key` dans l'Inspecteur pour voir comment elle marche :

1. **Visible seulement si…** contient la condition « non `cle_maison` » : la clé disparaît une fois prise.
2. **Au clic** liste trois actions, dans l'ordre : **Modifier une variable** (`cle_maison` mise à vrai), **Notification** (*Vous avez trouvé une clé !*) et **Rester sur cette carte**.
3. **Rester sur cette carte** garde le joueur dans la pièce au lieu d'aller vers **Mène à** : le clic ne change donc pas de pièce.
4. Pour fabriquer un autre objet de cette façon, cliquez sur **Ajouter une action** et choisissez parmi **Modifier une variable**, **Notification**, **Jouer un son**, **Rester sur cette carte**, **Aller à un label**, **Aller à une scène**, **Appeler un label (puis revenir ici)** et **Retour (fin d'un appel)**. Les flèches haut et bas réordonnent les actions. Sans **Rester sur cette carte**, la zone va vers **Mène à** après ses actions.

Déplacez la zone de la clé sur le vrai dessin d'une clé dans l'image (ou dessinez-en une nouvelle sur le bon objet).

### 5. La porte fermée

1. Sélectionnez la zone de la porte vers la chambre (nommée `locked_door_…`).
2. **Cliquable seulement si… (sinon grisée)** contient `cle_maison`. Tant que la clé n'est pas trouvée, la zone est grisée et ne réagit pas au clic ; ensuite elle fonctionne. **Visible seulement si…** la cacherait à la place.
3. Ajoutez un message pour le joueur : donnez à la zone une **Info-bulle** comme *Fermée à clé*, ou ajoutez une seconde zone sur la même porte qui n'apparaît que sans la clé, avec la condition **Visible seulement si…** réglée sur `!cle_maison` et une action **Notification** *La porte est fermée*.

### 6. Des zones à la forme de leur image

Un rectangle est rarement le contour exact d'un objet. Une zone peut suivre les pixels opaques d'un PNG :

1. Importez le PNG de l'objet (par exemple un vase sur fond transparent) dans la bibliothèque.
2. Sélectionnez une zone et réglez sa **Forme** sur **Image (pixels opaques)**. Choisissez l'**Image**.
3. Appuyez sur **Ajuster à l'image** pour la placer à sa taille propre, centrée, ou **Comme le décor** si le PNG est une copie transparente de tout le décor où seul l'objet est peint.
4. Le **Seuil d'opacité** décide à partir de quelle opacité un pixel réagit à la souris. Seuls les pixels de l'objet sont cliquables, pas les coins transparents.
5. Cochez **Dessiner l'image par-dessus le décor** pour afficher l'objet lui-même, et réglez une **Image au survol (au lieu de la lueur)** pour en montrer une version éclairée quand le curseur passe dessus.

### 7. Sortir de la maison

1. Ajoutez une zone sur la porte d'entrée.
2. Dans **Mène à**, cliquez sur **Aller à une scène…** et choisissez la scène suivante de votre histoire. Un label et un saut vers cette scène sont créés et la zone y pointe.
3. Jouez depuis le début avec Preview et parcourez toute la maison.

## Résultat

Une maison navigable où chaque pièce est une image avec des zones cliquables, une clé ramassée et mémorisée par une variable, une porte qui ne s'ouvre qu'avec elle, des objets qui réagissent à la forme exacte de leur dessin, et une sortie vers la suite de l'histoire.

## Pour aller plus loin

- Donnez à une zone un **Son au survol**, une **Apparence propre** (couleur, lueur, pulsation) ou activez l'**Indice au repos (scintillement)** pour suggérer où cliquer.
- Combinez des variables : plusieurs clés, un compteur d'inventaire, un drapeau qui change le contenu d'une pièce. Les champs **Visible seulement si…** et **Cliquable seulement si…** acceptent un éditeur simple avec plusieurs tests.
- Une zone peut appeler un petit morceau d'histoire (**Appeler un label (puis revenir ici)**) pour décrire un objet, puis revenir à la carte.
- La même étape **Zones** sert pour un menu, une carte du monde ou une énigme : elle ne se limite pas aux maisons.

## En cas de problème

- **Une zone n'est pas là où je l'ai dessinée** : dans l'Éditeur de vue, vérifiez que l'étape **Zones** (et non une réplique) est sélectionnée, puis utilisez **Sélection** pour la déplacer.
- **Je ne trouve pas la barre d'outils** : elle n'apparaît que lorsqu'une étape **Zones** est sélectionnée dans le panneau Scène.
- **Le joueur est coincé dans une pièce** : vérifiez qu'au moins une zone de la pièce est toujours visible et cliquable (sans condition) et possède un **Mène à**. Le panneau **Problèmes** signale les zones invalides (forme vide, action de variable sans variable), pas une carte sans issue.
- **Cliquer sur la clé change de pièce** : l'action **Rester sur cette carte** manque dans **Au clic**.
- **Une zone Image ne réagit pas** : le PNG doit avoir un fond transparent et le **Seuil d'opacité** ne doit pas dépasser l'opacité du dessin.
