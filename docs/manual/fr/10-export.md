# Exporter votre jeu

Un export produit un jeu que les joueurs lancent **sans rien installer** : un `.zip` pour Windows, un `.tar.gz` pour Linux, ou un dossier à mettre sur un site web.

## Depuis l'éditeur

Choisissez **Fichier > Exporter le jeu** pour ouvrir le panneau Export.

1. **Profil d'export** : le nom du jeu, un **identifiant** (le dossier où sont rangées les sauvegardes des joueurs : minuscules, chiffres, `-` et `_` ; ne le changez plus après publication), la version, une icône (un PNG carré, 256x256 conseillé), la langue par défaut et la compression. Enregistrez les profils pour les garder avec le projet ; vous pouvez en avoir plusieurs (par exemple `demo` et `release`).
2. **Cibles** : Windows, Linux et Web. Une cible dont le player est introuvable est signalée.
3. **Dossier de sortie** : `export/` dans le projet par défaut.
4. **Exporter** : une barre de progression et un journal montrent chaque étape et les avertissements éventuels (assets inutilisés, traductions incomplètes). **Annuler** l'arrête. L'histoire est enregistrée d'abord, et les erreurs du panneau Problèmes bloquent l'export.
5. À la fin, ouvrez le dossier, et pour le web utilisez **Tester dans le navigateur**.

## Ce que reçoit le joueur

- **Windows** : un seul `.exe` contenant le jeu, et un court README.
- **Linux** : un seul exécutable ; un pilote graphique (Vulkan ou OpenGL) est nécessaire.
- **Web** : un dossier à mettre tel quel sur n'importe quel hébergeur statique (itch.io, GitHub Pages, Netlify...). Le jeu se charge scène par scène pour démarrer vite. Le plein écran est dans les préférences ou sur F11. La partie est sauvegardée quand l'onglet est masqué.

Les sauvegardes sont rangées sous l'identifiant du jeu dans le dossier de données de l'utilisateur (le stockage du navigateur pour la version web).

## Android

Le player du jeu existe en version Android, avec commandes tactiles et sauvegarde automatique à la mise en pause de l'application. L'export d'un paquet Android directement depuis l'éditeur n'est pas encore disponible.

## Ligne de commande

L'outil `orbit` fait la même chose depuis un terminal : `orbit export windows --project mon_jeu --out dist`, ainsi que `linux`, `web` et `all`.
