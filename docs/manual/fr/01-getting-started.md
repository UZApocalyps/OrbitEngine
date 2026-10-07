# Premiers pas

OrbitEngine permet de créer un visual novel sans écrire de code. Vous écrivez des répliques, proposez des choix, affichez des personnages et des décors, et l'éditeur joue le résultat au fur et à mesure.

## Créer votre premier projet

1. Ouvrez l'éditeur. La page d'accueil propose **Nouveau projet** (Ctrl+N).
2. Saisissez un nom et choisissez le dossier qui contiendra votre projet.
3. Choisissez un **modèle** :
   - **Vide** : une scène et deux répliques, pour partir de zéro.
   - **Démarrage visual novel** : deux scènes, deux personnages, un choix, une variable, des écrans de menu personnalisés et un splash. Vous pouvez y jouer tout de suite, puis le réécrire.
4. Appuyez sur **Créer**. Le projet s'ouvre.

Un projet est un dossier ordinaire. Votre histoire est le fichier `story.orbit`, vos images et sons sont dans `assets/`, vos écrans personnalisés dans `ui/`. Vous pouvez mettre tout le dossier sous gestion de versions ou le copier sur un autre ordinateur.

## Les fenêtres principales

- **Scène** : l'histoire sous forme de liste d'étapes, dans l'ordre de lecture. C'est là que vous écrivez la plupart du temps.
- **Graphe** : la même histoire dessinée en boîtes et en flèches, pratique pour voir où mènent les choix.
- **Inspecteur** : tous les réglages de l'étape sélectionnée.
- **Personnages** et **Variables** : les définitions utilisées par votre histoire.
- **Bibliothèque** : vos images et sons. Glissez des fichiers dans la fenêtre pour les ajouter.
- **Preview** : joue l'histoire avec le vrai moteur, et se recharge quand vous modifiez.
- **Problèmes** : vérifications en direct. Une erreur bloque l'export, un avertissement non. Les vérifications tournent en arrière-plan environ 300 ms après votre dernière modification. Désactivez-les avec **Affichage > Validation automatique** (utile sur les très gros projets) : la liste est alors estompée quand elle est périmée et se met à jour avec **Valider** (Ctrl+F7). L'export et la compilation valident toujours d'abord.

Chaque fenêtre peut être rouverte depuis le menu **Affichage**, et réorganisée en faisant glisser son onglet.

## Enregistrer et annuler

Ctrl+S enregistre. Ctrl+Z annule et Ctrl+Y rétablit, aussi loin que nécessaire. Si l'éditeur se ferme brutalement, il propose de restaurer votre travail non enregistré à la réouverture du projet.

## Et ensuite

Chaque chapitre de ce manuel traite d'un sujet. Ouvrez **Aide > Manuel** à tout moment (ou appuyez sur F1). Le manuel suit la langue de l'éditeur.
