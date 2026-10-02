# Serveur MCP d'OrbitEngine (mode headless, M3f)

`orbit mcp --project <dossier|fichier.ron>` expose un projet OrbitEngine à une IA via le [Model Context Protocol](https://modelcontextprotocol.io) : l'IA lit et modifie l'histoire (scènes, répliques, choix, personnages, variables, assets) comme un collaborateur, sans interface graphique ni GPU.

* Transport : **stdio** (le client lance le processus). Rien d'autre que le protocole n'est écrit sur la sortie standard ; les diagnostics vont sur la sortie d'erreur.
* Implémentation : SDK Rust officiel **`rmcp` 3.5.0** (licence Apache-2.0, aucune nouvelle exception dans `deny.toml`), crate `orbit_mcp`. Versions de protocole négociées par `rmcp` (dont 2025-06-18).
* **Toute modification passe par l'API de commandes d'`orbit_project`** (`Project::execute` / `group`) : jamais d'écriture directe du RON. Une commande refusée ne laisse aucune modification partielle.
* Le MCP n'est jamais embarqué dans le player (`xtask check-arch` vérifie que `orbit_player` ne dépend ni d'`orbit_mcp` ni de `rmcp`).

## Brancher Claude Code

Compilez le binaire (`cargo build --release -p orbit_cli`, produit `target/release/orbit`), puis :

```sh
claude mcp add orbit -- /chemin/vers/orbit mcp --project /chemin/vers/mon_jeu
```

Ou, dans un fichier `.mcp.json` à la racine du dépôt de l'IA (ou de votre projet) :

```json
{
  "mcpServers": {
    "orbit": {
      "command": "/chemin/vers/orbit",
      "args": ["mcp", "--project", "/chemin/vers/mon_jeu"]
    }
  }
}
```

Sous Windows, utilisez `orbit.exe` et des chemins avec `\\` échappés (ou des `/`). `--project` accepte un dossier de projet (celui qui contient `orbit.project.ron` et `story.ron`), un dossier vide (un projet est alors créé) ou directement un fichier d'histoire `.ron` (son dossier devient la racine du projet).

Essayer sans client : `orbit mcp --project mon_jeu` attend des messages JSON-RPC (un par ligne) sur l'entrée standard.

## Modèle de travail

* **Une sauvegarde après chaque appel d'outil réussi** (écriture atomique : fichier temporaire, flush, renommage). Le fichier sur disque est toujours une histoire valide pour `orbit validate`, sauf si l'IA a volontairement laissé l'histoire dans un état invalide (l'outil `validate` le dit). L'outil `save` force une écriture.
* **Une entrée d'undo par appel d'outil** (libellé « MCP : <outil> »), y compris pour les outils de plugins, groupés par `Project::group`. `undo` / `redo` sont des outils ; ils sont sauvegardés eux aussi.
* **Appel atomique** : si un appel échoue (référence inconnue, conflit de version…), tout ce qu'il avait déjà appliqué est annulé, et l'erreur est renvoyée comme **résultat d'outil en erreur** (`isError: true`) avec un texte `categorie: message` que le modèle peut lire et corriger. Catégories : `invalid_arguments`, `not_found`, `version_conflict`, `refused`, `outside_project`, `io_error`, `plugin_error`, `internal_error` (une panique est interceptée : l'appel n'est pas appliqué).
* **Conflits de version par nœud** : `read_scene` affiche la version de chaque nœud ; `edit_dialogue`, `delete_node` et `move_node` acceptent `expected_version` et échouent avec `version_conflict` si le nœud a changé depuis. (Dans ce mode headless, le serveur est le seul à écrire ; la protection devient utile en mode live, M4, quand l'utilisateur édite en même temps.)
* Les nœuds, scènes, personnages et assets se désignent par **id** (ULID de 26 caractères) ; scènes, personnages, assets et labels acceptent aussi un **nom** quand il est sans ambiguïté.
* Les **cibles de saut sont des labels** : créer d'abord les labels (`add_command` `type: label`), puis les choix et sauts qui les visent. Pour insérer un choix avant ses labels, utiliser `after` (id d'un nœud, ou `start` / `end`) ou `index`.
* Les conditions et valeurs utilisent le langage d'expressions du moteur : `trust > 2 && !met_bob`, `route == "forest"`, `score + 1`.

## Outils

Les schémas d'entrée sont générés depuis les types Rust (`schemars`), sans `$ref`, et refusent les champs inconnus (une faute de frappe est une erreur claire). Annotations MCP : `readOnlyHint` pour la lecture, **`destructiveHint: true` pour `delete_scene` et `delete_node`** (les autres éditions sont annulables et additives ou ciblées).

| Outil | Rôle |
|---|---|
| `list_scenes` | Scènes (id, nom, nombre de nœuds). |
| `read_scene` | Lignes numérotées d'une scène : index, id du nœud, version, contenu lisible (répliques, choix et labels visés, commandes). |
| `create_scene`, `rename_scene` | Crée / renomme une scène (noms uniques). |
| `delete_scene` | **Destructif.** Supprime la scène et ses nœuds ; refusé tant qu'un nœud d'une autre scène saute vers elle (saut, option, saut de scène). |
| `add_dialogue` | Réplique (locuteur, texte, `expression` + `position` optionnelles qui insèrent un `show` avant la réplique, avec `scale`, `flip_x`, `flip_y` ; position d'insertion `after` / `index`). `position` est un nom (`left`, `center`, `right`, position du projet) ou des coordonnées libres `{"x": 0.4, "y": 1.0}`. |
| `edit_dialogue` | Change le texte et/ou le locuteur ; `expected_version`. |
| `add_choice` | Choix : options (texte, label cible, condition optionnelle). |
| `add_command` | Décor, `show` (position nommée ou libre `{"x", "y"}`, `scale` / `scale_y`, `flip_x` / `flip_y` ; réafficher un personnage déjà présent met à jour sa position, son échelle et son miroir) / `hide`, musique, son, transition (sur décor / show / hide), attente, `set_var`, `if`, `jump`, `jump_scene` (saut vers une autre scène, éventuellement vers une de ses étiquettes ; les scènes ne s'enchaînent jamais d'elles-mêmes), `label`, `call` / `return`, `end`, nœud de plugin (`custom`). |
| `delete_node` | **Destructif.** Supprime un nœud ; refusé tant qu'un autre nœud saute vers lui. |
| `move_node` | Déplace un nœud (dans la scène ou vers une autre). |
| `list_characters`, `create_character`, `edit_character` | Personnages, couleur, expressions (nom → asset image) et `default_scale` (échelle par défaut, 0,05 à 10, 1 = taille native). |
| `list_variables`, `declare_variable` | Variables déclarées (booléen, nombre, texte ; portée `game` ou `persistent`). |
| `list_assets`, `import_asset` | Manifeste d'assets ; enregistre un fichier **déjà situé dans le dossier du projet** (copié sous `assets/` s'il n'y est pas). |
| `validate` | Même contrôle que `orbit validate` (+ fichiers d'assets, validateurs de plugins). |
| `simulate` | Joue l'histoire sans GPU (`orbit_vm`) avec une liste de choix (index ou texte) et renvoie la transcription : l'IA vérifie ainsi chaque branche. S'arrête en listant les options si la liste de choix est épuisée. |
| `undo`, `redo`, `save` | Historique et sauvegarde explicite. |
| `<plugin_id>.<outil>` | Outils fournis par les plugins installés (`McpToolDef` d'`orbit_plugin_api`). |

### Ressources

`orbit://project/manifest`, `orbit://project/scenes`, `orbit://project/characters`, `orbit://project/assets`, `orbit://project/variables`, `orbit://project/validation` (rapport), `orbit://schema/ir` (JSON Schema de l'IR, généré depuis les types de `orbit_core`) et `orbit://scene/<id>` (une par scène).

### Exemple de session

```
create_character {"name": "Alice"}
create_scene {"name": "intro"}
add_dialogue {"scene": "intro", "speaker": "Alice", "text": "Par où ?"}
add_command  {"scene": "intro", "command": {"type": "label", "name": "gauche"}}
add_dialogue {"scene": "intro", "speaker": "Alice", "text": "Gauche, donc."}
add_command  {"scene": "intro", "command": {"type": "label", "name": "droite"}}
add_dialogue {"scene": "intro", "speaker": "Alice", "text": "Droite, donc."}
add_choice   {"scene": "intro", "after": "<id de « Par où ? »>",
              "options": [{"text": "Gauche", "target": "gauche"},
                          {"text": "Droite", "target": "droite"}]}
validate {}
simulate {"choices": [0]}      # puis {"choices": ["Droite"]}
```

## Sécurité

* **Périmètre = le dossier du projet.** Tout chemin reçu passe par `Sandbox::resolve` : chemins vides ou contenant NUL refusés, tout composant `..` refusé, chemins absolus acceptés seulement s'ils tombent dans le projet, **liens symboliques et jonctions résolus** (un lien qui sort du dossier, ou un lien pendant, est refusé), vérification sur le chemin canonique. Aucune opération ne lit ni n'écrit hors du dossier (`import_asset` ne copie qu'un fichier déjà dans le projet ; le nom de destination est assaini).
* **Opérations destructrices signalées** (`destructiveHint`) : suppression de scène et de nœud. Toutes les modifications sont annulables (`undo`).
* **Journal des actions** : `<projet>/.orbit/mcp.log`, une ligne par appel (horodatage Unix en ms, outil, succès ou erreur, arguments et résultat tronqués). Les erreurs y sont aussi.
* **Aucune panique sur entrée invalide** : arguments mal typés, ids invalides, index géants, JSON absurde → erreur structurée, projet inchangé (test de robustesse + interception des paniques).
* Le serveur écoute sur l'entrée standard du processus que le client a lancé : il n'ouvre aucun port réseau. (Le mode live HTTP de l'éditeur, M4, sera sur 127.0.0.1, désactivé par défaut, avec jeton.)
* Limite de taille d'import : 256 Mio par fichier.

## Dettes et limites (M3f)

* Pas de surveillance des fichiers : si le projet est modifié sur disque pendant que le serveur tourne (éditeur ouvert en même temps), le serveur n'en sait rien et écrasera à la prochaine sauvegarde. Le mode live (M4) donne un seul propriétaire du projet.
* Les plugins intégrés de l'éditeur (`orbit_editor`) ne sont pas chargés par `orbit mcp` : seuls les outils du cœur sont exposés tant qu'aucun plugin n'est installé (`HeadlessHost::install_plugin` existe pour les hôtes et les tests).
* Les assets sont enregistrés et copiés mais leur contenu n'est pas inspecté (un faux PNG est accepté ; `validate` ne vérifie que l'existence du fichier).
* Pas de suppression de personnage, de variable ni d'asset, ni d'édition des nœuds autres que les répliques (supprimer puis recréer) ; pas d'outil de build / export ni de capture d'écran de la preview (M4, mode live).
* Les libellés d'undo des commandes d'`orbit_project` sont en français ; ceux des appels MCP sont « MCP : <outil> ».
