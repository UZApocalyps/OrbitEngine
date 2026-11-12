# Tutoriel : votre premier visual novel

Ce tutoriel vous mène d'un éditeur vide à un petit jeu que vous pouvez envoyer à un ami, en une quinzaine de minutes. Vous allez créer un projet, un personnage à deux expressions, un décor, quelques répliques, un choix, une variable et une condition, puis jouer le résultat et l'exporter pour Windows.

## Objectif

Une courte histoire où le joueur peut partager son déjeuner avec une camarade ou l'ignorer. Le choix est mémorisé dans une variable, et la fin dépend de lui.

## Avant de commencer

- L'éditeur est installé et s'ouvre (voir *Premiers pas*).
- Vous avez deux images d'un personnage (PNG à fond transparent, par exemple `neutre.png` et `content.png`) et une image de décor (PNG ou JPEG). N'importe quelles images conviennent pour un premier essai : vous pourrez les remplacer plus tard.

## Étapes

### 1. Créer le projet

1. Sur la page d'accueil, cliquez sur **Nouveau projet** (ou appuyez sur Ctrl+N).
2. Dans la fenêtre **Nouveau projet**, saisissez un **Nom**, choisissez le **Dossier parent** avec **Parcourir…**, puis le modèle **Vide**.
3. Cliquez sur **Créer**. Le projet s'ouvre avec une scène nommée *Introduction* et deux répliques d'exemple.

### 2. Importer vos images

1. Dans le panneau **Assets**, cliquez sur **Importer** et choisissez vos trois images. Vous pouvez aussi les déposer sur la fenêtre de l'éditeur.
2. Avec le filtre **Images** de la bibliothèque, vérifiez qu'elles sont toutes là. Les fichiers sont copiés dans le dossier `assets/` du projet.

### 3. Créer un personnage

1. Ouvrez le panneau **Personnages** et cliquez sur **Nouveau**.
2. Remplissez **Nom affiché** (par exemple `Mia`) et choisissez une **Couleur** : elle teinte son nom dans la boîte de dialogue.
3. Sous **Expressions**, cliquez sur **Ajouter une expression**, nommez-la `neutre`, puis choisissez l'image avec **Fichier…**. Recommencez pour `content`.

### 4. Installer la scène

1. Dans le panneau **Scène**, sélectionnez la première réplique. Appuyez sur Ctrl+I pour ouvrir le menu d'insertion rapide, tapez `décor` et choisissez l'étape **Décor**, ou utilisez **Édition > Insérer > Insérer un décor**.
2. Dans l'**Inspecteur**, choisissez votre **Image** et, comme **Transition**, **Fondu enchaîné**.
3. Insérez ensuite une étape **Afficher un personnage**. Dans l'Inspecteur, choisissez le **Personnage** Mia, l'**Expression** `neutre` et la **Position** **Centre**.

### 5. Écrire le dialogue

1. Sélectionnez la dernière réplique et appuyez sur Ctrl+Entrée pour en ajouter une après elle. Tapez le texte, puis Entrée pour la réplique suivante (Maj+Entrée pour un retour à la ligne dans la même réplique, Échap pour arrêter d'écrire).
2. Pour chaque réplique, choisissez qui parle dans le champ **Personnage** de la ligne, ou laissez-le vide pour le narrateur.
3. Pour changer le visage de Mia en pleine conversation, insérez une autre étape **Afficher un personnage** avec l'expression `content`. Afficher un personnage déjà en scène ne fait que le mettre à jour.

Écrivez quatre ou cinq répliques, par exemple : Mia vous voit, dit qu'elle a oublié son déjeuner et demande ce que vous en pensez.

### 6. Ajouter un choix

1. Choisissez **Édition > Insérer > Insérer un choix**. L'étape arrive avec deux options, *Option A* et *Option B*.
2. Sélectionnez l'étape de choix. Dans l'Inspecteur, écrivez le texte de chaque option : `Partager ton repas` et `Ne rien dire`.
3. Pour chaque option, cliquez sur **Nouvelle étiquette** à côté de **Mène à**. Cela crée un point d'arrivée à la fin de la scène et y envoie l'option.
4. Sélectionnez chaque nouvelle étiquette dans la scène et renommez-la dans **Nom de l'étiquette** : `partage` pour la première, `apres` pour la seconde.

La scène se termine maintenant par deux étiquettes. L'exécution traverse une étiquette comme n'importe quelle étape : les répliques que vous écrirez après `partage` se prolongeront dans `apres`.

### 7. Mémoriser le choix avec une variable

1. Ouvrez le panneau **Variables** et cliquez sur **Nouvelle**. Nommez-la `amitie`, choisissez le type **Nombre entier** et laissez la **valeur initiale** à 0. Gardez la **Portée** sur **Partie**.
2. Sélectionnez l'étiquette `partage` et insérez après elle une étape **Modifier une variable**. Choisissez la **Variable** `amitie` ; dans **Nouvelle valeur**, prenez l'opération **ajouter** et la valeur 1.
3. Ajoutez en dessous une réplique de Mia contente : *« Merci, c'est vraiment gentil ! »*

### 8. Réagir au choix avec une condition

1. Sélectionnez l'étiquette `apres` et insérez après elle une étape **Condition**.
2. Dans l'Inspecteur, avec l'**Éditeur simple**, choisissez la variable `amitie`, le test **est supérieur ou égal à** et la valeur 1.
3. Pour **Si vrai, aller à**, cliquez sur **Nouvelle étiquette** et nommez-la `bonne_fin`. Laissez **Sinon** sur **Continuer à la ligne suivante**.
4. Sous l'étape Condition, écrivez la mauvaise fin (deux ou trois répliques), puis insérez une étape **Fin de l'histoire**.
5. Tout à la fin de la scène, après l'étiquette `bonne_fin`, écrivez la bonne fin et terminez par une autre étape **Fin de l'histoire**.

### 9. La jouer

1. Dans le panneau **Preview**, appuyez sur **Lecture** (F6). Cliquez sur l'image pour faire avancer l'histoire.
2. Essayez les deux options. Sélectionnez une étape et appuyez sur **Ici** (F7) pour repartir du milieu sans tout rejouer.
3. Ouvrez le panneau **Problèmes**. Une histoire bien construite affiche *Aucun problème détecté.* Cliquez sur un problème pour aller à l'étape fautive.

### 10. Exporter pour Windows

1. Enregistrez avec Ctrl+S, puis choisissez **Fichier > Import, export et traductions > Exporter le jeu…**.
2. Dans **Profil d'export**, écrivez le **Nom du jeu**, un **Identifiant** court (minuscules, chiffres, `-` et `_` ; ne le changez plus après publication) et la **Version**.
3. Sous **Cibles**, cochez **Windows (zip)**. Une cible marquée *(template introuvable)* n'a pas de lecteur installé : voir le chapitre *Exporter votre jeu*.
4. Cliquez sur **Exporter**. Quand le journal annonce que l'export est terminé, cliquez sur **Ouvrir le dossier** : le `.zip` s'y trouve. N'importe qui peut le décompresser et jouer, sans rien installer.

## Résultat

Vous avez un jeu complet et jouable : une scène avec un décor, un personnage à deux expressions, un choix, une variable qui s'en souvient, deux fins et un `.zip` à partager.

## Pour aller plus loin

- Ajoutez de la musique : insérez une étape **Jouer une musique** au début (*Écrire l'histoire*).
- Utilisez plusieurs variables et testez-les ensemble dans la condition d'une étape Condition.
- Changez l'apparence des menus et de la boîte de dialogue : tutoriel *Tutoriel : personnaliser l'interface du jeu*.
- Faites parler votre jeu dans d'autres langues : tutoriels *Succès, notifications et traductions* et *Publier votre jeu*.

## En cas de problème

- **Une erreur rouge dans le panneau Problèmes** : les erreurs bloquent l'export. Cliquez dessus pour atteindre l'étape. Causes fréquentes : une option de choix qui ne mène nulle part (**Mène à** vide) ou une étiquette supprimée.
- **« La partie se termine ici »** : avertissement indiquant qu'une scène n'a ni **Fin de l'histoire**, ni saut, ni retour. Ajoutez-en un à la fin de chaque branche.
- **L'histoire passe de `partage` directement à la mauvaise fin** : les étiquettes n'arrêtent pas l'histoire. Vérifiez l'ordre des étapes dans le panneau Scène, ou regardez les flèches du panneau **Graphe**.
- **Le personnage est invisible** : il faut une étape **Afficher un personnage** avant ses répliques, et l'expression doit avoir une image.
- **L'aperçu est vide** : appuyez d'abord sur Lecture (F6) ; si le message dit qu'aucun GPU n'a pu être initialisé, l'aperçu n'est pas disponible sur cet ordinateur, mais l'export fonctionne quand même.
