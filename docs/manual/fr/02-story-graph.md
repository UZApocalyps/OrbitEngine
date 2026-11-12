# Écrire l'histoire

Une histoire est faite de **scènes**. Une scène est une liste d'**étapes** que le jeu joue l'une après l'autre : une réplique, un changement de décor, un choix, etc.

## L'éditeur de scène

Ajoutez des étapes avec **Édition > Insérer**, les boutons de l'éditeur de scène ou de l'Explorateur, ou le menu rapide (Ctrl+I). Ctrl+Entrée ajoute une réplique. Le menu d'insertion est organisé en sous-menus par catégorie. Tous les types d'étapes :

**Dialogue**

- **Réplique** : qui parle (un personnage, ou personne pour la narration) et le texte.
- **Choix du joueur** : deux options ou plus. Chaque option mène à une **étiquette** et peut avoir une condition.
- **Notification** : un court message affiché dans un coin de l'écran pendant que l'histoire continue (voir *Notifications* plus bas).

**Scène**

- **Décor**, **Afficher un personnage** / **Masquer un personnage**, **Attendre**. Les transitions (fondu enchaîné, fondu au noir) se règlent sur les décors et sur l'affichage ou le masquage des personnages.

**Effets**

- **Effet** (déplacer, secouer, zoomer, flash, teinte), **Chronologie** (pistes de clés sur la caméra, l'écran ou un personnage), **Fond en parallaxe**, **Particules** et **Arrêter les particules**. Voir les chapitres *Personnages et animations* et *Courbes*.

**Écrans**

- **Ouvrir un écran** / **Fermer un écran** : afficher un écran du projet (un inventaire, un HUD) depuis l'histoire, en attendant ou non que le joueur le ferme. Voir le chapitre *Interface*.

**Son**

- **Jouer une musique**, **Arrêter la musique**, **Jouer un son**.

**Logique**

- **Modifier une variable** et **Condition** (si) : voir le chapitre *Variables*.
- **Script (Rhai)** : un peu de code pour une logique trop lourde pour le graphe (voir *Scripts*).
- **Débloquer un succès** (voir *Succès* dans le chapitre Interface).
- **Zones cliquables (point & click)** : une carte où le joueur clique sur des zones (voir plus bas).

**Navigation**

- **Étiquette** : un endroit nommé dans une scène, cible d'un choix ou d'un saut.
- **Saut vers une étiquette**, **Saut de scène** (les scènes ne s'enchaînent jamais seules, indiquez donc où l'histoire continue).
- **Appeler une routine** / **Retour de routine** : réutiliser un petit morceau d'histoire depuis plusieurs endroits.
- **Fin de l'histoire** : l'histoire s'arrête.

**3D** (expérimental, seulement quand *Affichage > Options > Fonctionnalités 3D expérimentales* est coché) : **Afficher une scène 3D**, **Caméra 3D**, **Masquer la scène 3D**. Voir le chapitre *Scènes 3D*.

**Plugins** : les étapes ajoutées par les plugins du projet (voir *Plugins Rust*).

Sélectionnez une étape pour modifier tous ses réglages dans l'Inspecteur (clic droit, **Modifier**, pour l'ouvrir directement). Ctrl+C, Ctrl+X, Ctrl+V, Ctrl+D et Alt+Haut / Alt+Bas copient, coupent, collent, dupliquent et déplacent les étapes.

## Zones cliquables (point & click)

Une étape **Zones cliquables** transforme le décor en carte : l'histoire s'y arrête jusqu'à ce que le joueur clique sur une zone, puis va là où la zone mène. Servez-vous-en pour une maison à explorer, une pièce pleine d'objets à examiner, une carte du monde.

L'étape a les réglages de la **carte** : un décor propre facultatif (**Afficher son propre décor** ; sans lui, le décor courant est utilisé) et une **apparence** partagée par toutes les zones (couleur, remplissage, largeur et intensité de la lueur, vitesse de pulsation, scintillement au repos, curseur main). Chaque zone a :

- un **nom**, une **infobulle** facultative affichée au survol, et **Mène à** : l'étiquette où va l'histoire quand on clique la zone. **Aller à une scène...** crée une étiquette et un saut vers une autre scène, et y pointe la zone.
- une **forme** : rectangle, ellipse, polygone, ou **Image (pixels opaques)** : une image du projet, posée sur le décor, dont seuls les pixels opaques réagissent (un objet posé sur la scène). Pour une image, **Ajuster à l'image** lui donne sa taille propre, **Comme le décor** l'étire sur l'écran (un objet peint sur une copie transparente du décor), le **seuil d'opacité** décide quels pixels comptent et **Dessiner l'image par-dessus le décor** l'affiche. La lueur suit le contour de l'objet.
- **Visible seulement si...** et **Cliquable seulement si... (sinon grisée)** : des conditions sur des variables.
- une **image de survol** (qui remplace la lueur), un **son de survol** et une **apparence propre**.
- des actions **Au clic**, exécutées avant que l'histoire continue : **Modifier une variable** (sans code), **Notification**, **Jouer un son**, puis l'une de **Rester sur cette carte** (par exemple ramasser un objet, puis la carte est réaffichée), **Aller à un label**, **Aller à une scène**, **Appeler un label (puis revenir ici)** ou **Retour (fin d'un appel)**. Sans action de flux, la zone va à **Mène à**.

Dessinez les zones directement dans le panneau **Éditeur de vue** : choisissez **Rectangle**, **Ellipse** ou **Polygone** dans la barre *Zones* et glissez sur l'image (un polygone : cliquez les points, puis le premier à nouveau, double-cliquez ou appuyez sur Entrée). Avec l'outil *Sélection*, glissez une zone pour la déplacer, glissez ses coins ou ses points pour la remodeler, double-cliquez sur le bord d'un polygone pour ajouter un point, clic droit sur un point pour le retirer. Suppr supprime la zone. Chaque geste est une étape d'annulation.

Dans le jeu, une zone au repos scintille doucement, une zone survolée reçoit un halo pulsé et le curseur main, et son infobulle apparaît. On peut choisir les zones au clavier (Tab, flèches et chiffres) et à la manette, et **V** affiche toutes les zones d'un coup (la manette a une action *afficher les zones*, Select / Vue par défaut, qui peut être assignée à un autre bouton).

Dans le format texte, une carte s'écrit :

```
hotspots bg "assets/hall.png"
    spot rect (0.1, 0.2, 0.25, 0.5) "La porte" -> dehors name porte
        do set cle = 1
        do notify "Vous avez trouvé une clé !"
        do stay
```

### L'assistant de carte de navigation

**Édition > Insérer > Nouvelle carte de navigation (maison)...** (aussi dans la palette) écrit toute une maison en une étape d'annulation : vous listez les pièces, choisissez un décor pour chacune, cochez les pièces reliées, et l'assistant crée une étiquette et une carte par pièce, une zone de porte vers chaque pièce reliée (un rectangle : placez-le sur la vraie porte dans l'Éditeur de vue), une zone **Retour** facultative et un exemple de porte fermée à clé par une variable.

## Notifications

L'étape **Notification** affiche un court message dans un coin de l'écran, et l'histoire continue aussitôt. Elle a un texte (balises et `[variable]` permises), et une icône, un son, une durée et un style facultatifs. Son aspect se règle dans l'onglet **Notifications** de l'éditeur d'interface (voir le chapitre *Interface*). Les scripts font la même chose avec `notify(...)`.

## Le Graphe

Le Graphe montre les mêmes étapes sous forme de boîtes reliées par des flèches. C'est le meilleur endroit pour vérifier que chaque choix mène quelque part et repérer les impasses. Déplacez les boîtes pour ranger l'affichage ; cela ne change pas l'histoire.

## Vérifier votre travail

Le panneau **Problèmes** vous avertit par exemple d'un choix qui ne mène nulle part, d'une scène qui ne se termine jamais ou d'une image manquante. Cliquez sur un problème pour aller à l'étape fautive ; F8 passe au suivant. Revalidez avec Ctrl+F7.

## L'essayer

Le panneau **Preview** joue l'histoire comme le verra un joueur (F6). Utilisez **Jouer depuis ici** (F7) sur une étape pour démarrer avec l'état qu'aurait un joueur à cet endroit (personnages en scène, variables définies).

## Écrire en texte

La **Vue texte** montre l'histoire comme un script que l'on peut taper (voir le chapitre *Vue texte*). Le texte et le graphe décrivent toujours la même histoire.
