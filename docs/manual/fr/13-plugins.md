# Plugins Rust

Les plugins ajoutent de nouveaux types d'étapes et de nouvelles actions d'interface à un jeu. Ils sont écrits en Rust par un programmeur ; vous n'en avez pas besoin pour faire un jeu. Ce chapitre explique comment en utiliser un, par exemple le plugin de dés fourni avec le moteur (`examples/plugins/dice`). Les plugins qui étendent l'éditeur lui-même (Studio IA...) sont décrits dans *Plugins de l'éditeur*.

## Ajouter un plugin à un projet

1. Copiez le dossier du plugin dans le dossier `plugins` de votre projet (créez-le si besoin) : `plugins/dice/`.
2. Ses étapes apparaissent alors dans le menu d'insertion de l'éditeur sous **Plugins** (quand l'éditeur connaît le plugin), avec un inspecteur fait des champs que le plugin déclare : nombres avec leurs limites, cases à cocher, listes de choix et sélecteurs de variable.
3. Une étape de plugin range ses résultats dans des **variables** de l'histoire que vous déclarez vous-même (pour les dés : une variable entière qui reçoit le total). C'est ce qui fait fonctionner les sauvegardes, les chargements et le retour en arrière : le plugin ne garde rien pour lui.

## Installer un plugin depuis le Store

Les plugins de jeu publiés dans un catalogue s'installent sans copier de dossier :

1. Ouvrez le projet, puis **Outils > Extensions > Plugins…**, onglet **Store**, et choisissez **Plugins de jeu (projet)** en haut de l'onglet. Chaque plugin porte un badge **Jeu**, sa source (Officiel, Tiers, Non signé), la version proposée pour ce moteur et son état dans le projet.
2. Cliquez sur **Installer dans le projet**. Le fichier est téléchargé, sa taille, son empreinte SHA-256 et sa signature sont vérifiées, puis ses sources sont copiées dans `plugins/<nom>/` du projet, et le plugin est ajouté à la liste des plugins requis de `orbit.project.ron`. Rien n'est installé dans le dossier des plugins de l'éditeur.
3. Avant une installation depuis une source qui n'est pas la source officielle, l'éditeur demande confirmation : un plugin de jeu est du code Rust compilé et exécuté sur votre ordinateur à l'export (scripts de build compris) puis dans le jeu de vos joueurs. N'installez que des plugins de confiance.

L'onglet **Installés** a une section **Plugins de jeu de <projet>** : les crates de `plugins/`, leur source (ou « ajouté à la main »), les mises à jour (proposées uniquement par la source d'installation), **Désinstaller** et **Ouvrir le dossier**. Elle indique aussi :

- si l'éditeur connaît les types de nœuds du plugin. Il ne les connaît que si sa partie éditeur est chargée (un plugin d'éditeur du même identifiant, à installer depuis le Store, onglet **Plugins d'éditeur**). Sinon ses étapes restent affichées en lecture seule, comme un plugin manquant, mais le jeu exporté les exécute ;
- si l'export pourra construire le player : il faut la toolchain Rust et les sources du moteur (`ORBIT_ENGINE_SRC`). Sans elles le plugin s'installe quand même, mais l'export échouera : l'éditeur le dit. Si les sources du moteur sont introuvables à l'installation, réinstallez le plugin après avoir défini `ORBIT_ENGINE_SRC`.

**Vos modifications ne sont jamais écrasées en silence.** Si vous avez modifié les fichiers d'un plugin installé par le Store, ou si `plugins/` contient déjà une crate écrite à la main au même endroit ou fournissant le même identifiant, l'installation, la mise à jour et le retrait s'arrêtent et listent les fichiers concernés. **Remplacer (garder une sauvegarde)** déplace l'ancien dossier dans `.orbit/plugin-backups/` du projet. Compiler le plugin (`target/`, `Cargo.lock`) ne compte pas comme une modification.

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

Un plugin est une crate Rust qui dépend d'`orbit_plugin_api` (version d'API 1, mêmes sources du moteur). Elle implémente `Plugin` (côté éditeur : types de nœuds et leurs champs) et `RuntimePlugin` (handlers), déclare ses identifiants dans `Cargo.toml` sous `[package.metadata.orbit] plugins = ["votre.id"]` et exporte `orbit_plugins()`. Pour le publier dans un catalogue : `cargo xtask game-plugin pack <dossier> --sign <clé>` produit un paquet de sources signé (`.orbitsrc`), et `cargo xtask game-plugin index --index index.json` l'ajoute au catalogue (voir `docs/plugin-store.md`). Ses dépendances vers le moteur se font par chemin (`../crates/orbit_*`), l'éditeur les fait pointer vers `ORBIT_ENGINE_SRC` à l'installation. Un handler lit les variables, tire dans le générateur aléatoire seedé de la partie et renvoie des opérations sur les variables et des événements ; il peut faire patienter l'histoire quelques secondes. Il ne doit garder aucun état de jeu pour lui : tout ce qui doit survivre à une sauvegarde ou à un retour en arrière va dans des variables.

Un gestionnaire peut aussi afficher une notification (un toast avec un texte, et éventuellement une icône, un son, une durée et un style) : elle s'affiche une fois le gestionnaire réussi, comme la fonction `notify(...)` des scripts.
