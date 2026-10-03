# Écrire l'histoire

Une histoire est faite de **scènes**. Une scène est une liste d'**étapes** que le jeu joue l'une après l'autre : une réplique, un changement de décor, un choix, etc.

## L'éditeur de scène

Ajoutez des étapes avec les boutons de l'éditeur de scène ou de l'Explorateur. Les plus courantes :

- **Réplique** (dialogue) : qui parle (un personnage, ou personne pour la narration) et le texte.
- **Choix** : deux options ou plus. Chaque option mène à une **étiquette**.
- **Étiquette** : un endroit nommé dans une scène, cible d'un choix ou d'un saut.
- **Saut** : aller à une étiquette. **Saut de scène** : aller dans une autre scène (les scènes ne s'enchaînent jamais seules, indiquez donc où l'histoire continue).
- **Décor**, **Afficher** / **Masquer** un personnage, **Musique**, **Son**, **Attendre**.
- **Condition** (si) : aller d'un côté ou de l'autre selon une variable.
- **Appel** / **Retour** : réutiliser un petit morceau d'histoire depuis plusieurs endroits.
- **Ouvrir un écran** / **Fermer un écran** : afficher un écran du projet (un inventaire, un HUD) depuis l'histoire, en attendant ou non que le joueur le ferme. Voir le chapitre Interface.
- **Fin** : l'histoire s'arrête.

Sélectionnez une étape pour modifier tous ses réglages dans l'Inspecteur. Les transitions (fondu enchaîné, fondu au noir) se règlent sur les décors et sur l'affichage ou le masquage des personnages.

## Le Graphe

Le Graphe montre les mêmes étapes sous forme de boîtes reliées par des flèches. C'est le meilleur endroit pour vérifier que chaque choix mène quelque part et repérer les impasses. Déplacez les boîtes pour ranger l'affichage ; cela ne change pas l'histoire.

## Vérifier votre travail

Le panneau **Problèmes** vous avertit par exemple d'un choix qui ne mène nulle part, d'une scène qui ne se termine jamais ou d'une image manquante. Cliquez sur un problème pour aller à l'étape fautive.

## L'essayer

Le panneau **Preview** joue l'histoire comme le verra un joueur. Utilisez **Jouer depuis ici** sur une étape pour démarrer avec l'état qu'aurait un joueur à cet endroit (personnages en scène, variables définies).
