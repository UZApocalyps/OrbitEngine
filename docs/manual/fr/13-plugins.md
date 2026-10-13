# Plugins Rust

Les plugins ajoutent de nouveaux types d'étapes et de nouvelles actions d'interface à un jeu. Ils sont écrits en Rust par un programmeur ; vous n'en avez pas besoin pour faire un jeu. Ce chapitre explique comment en utiliser un, par exemple le plugin de dés fourni avec le moteur (`examples/plugins/dice`). Les plugins qui étendent l'éditeur lui-même (Studio IA...) sont décrits dans *Plugins de l'éditeur*.

## Ajouter un plugin à un projet

1. Copiez le dossier du plugin dans le dossier `plugins` de votre projet (créez-le si besoin) : `plugins/dice/`.
2. Ses étapes apparaissent alors dans le menu d'insertion de l'éditeur sous **Plugins** (quand l'éditeur connaît le plugin), avec un inspecteur fait des champs que le plugin déclare : nombres avec leurs limites, cases à cocher, listes de choix et sélecteurs de variable.
3. Une étape de plugin range ses résultats dans des **variables** de l'histoire que vous déclarez vous-même (pour les dés : une variable entière qui reçoit le total). C'est ce qui fait fonctionner les sauvegardes, les chargements et le retour en arrière : le plugin ne garde rien pour lui.

## Actions d'interface

Un bouton d'écran peut appeler un plugin avec l'action **Personnalisée** (Custom). Son nom est l'identifiant du plugin suivi de l'action, par exemple `orbit.dice.reroll`, et ses arguments sont donnés en texte (`total`, `6`).

## Exporter

Un projet avec des plugins est exporté avec un player construit pour lui : le panneau Export affiche un encadré **Plugins** avec les crates de plugins trouvées et indique si une toolchain Rust est installée.

- Installez Rust depuis https://rustup.rs si le panneau indique qu'elle manque.
- Le premier export construit le player (plusieurs minutes) ; les suivants sont beaucoup plus rapides.
- Pour l'instant, seule la plateforme sur laquelle tourne l'éditeur (Windows ou Linux) peut être exportée avec des plugins. Les exports web et Android ne sont pas encore disponibles.
- Un export est refusé si l'histoire utilise un plugin absent de `plugins/`.

## Plugins manquants

Si un projet utilise un plugin non installé, ses étapes sont conservées telles quelles et affichées en lecture seule, et le panneau Problèmes les liste. En jeu, une telle étape est ignorée avec un avertissement.

## Pour les programmeurs

Un plugin est une crate Rust qui dépend d'`orbit_plugin_api` (version d'API 1, mêmes sources du moteur). Elle implémente `Plugin` (côté éditeur : types de nœuds et leurs champs) et `RuntimePlugin` (handlers), déclare ses identifiants dans `Cargo.toml` sous `[package.metadata.orbit] plugins = ["votre.id"]` et exporte `orbit_plugins()`. Un handler lit les variables, tire dans le générateur aléatoire seedé de la partie et renvoie des opérations sur les variables et des événements ; il peut faire patienter l'histoire quelques secondes. Il ne doit garder aucun état de jeu pour lui : tout ce qui doit survivre à une sauvegarde ou à un retour en arrière va dans des variables.
