# Serveur MCP d'OrbitEngine (headless M3f, live dans l'éditeur M4)

`orbit mcp --project <dossier|fichier.ron>` expose un projet OrbitEngine à une IA via le [Model Context Protocol](https://modelcontextprotocol.io) : l'IA lit et modifie l'histoire (scènes, répliques, choix, personnages, variables, assets) comme un collaborateur, sans interface graphique ni GPU.

Deux modes, mêmes outils : **headless** (`orbit mcp`, ci-dessous) et **live** (l'éditeur ouvert expose le MCP en HTTP, voir « Mode live » plus bas).

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
| `list_scenes3d`, `read_scene3d`, `validate_scene3d` | Scènes 3D du projet (assets `scene3d`) : liste (caméras, nombre de nœuds, erreurs), lecture (nœuds avec transformation en angles d'Euler et quaternion, lumières, caméras nommées, matériaux, environnement, `ron` complet) et validation (références, cycles, noms de caméra, NaN, modèles et textures absents à côté de la scène, plus les `show_scene3d` / `camera3d` de l'histoire qui nomment une caméra inexistante). Lecture seule. |
| `rig_list`, `rig_read`, `rig_sample` | Rigs 2D (assets `rig`, fichiers `.rig.ron`, déclarés ou non dans `assets/rigs`) : liste (os, slots, skins, clips avec durée, images et images manquantes, IK, problèmes, expressions qui l'utilisent), lecture (structure, problèmes avec sévérité, RON) et pose d'un clip à un temps donné (`rig_sample` : transformations locales et monde par os, slots, ordre de dessin ; le temps suit les modes `loop` / `once` / `ping_pong` du runtime). Lecture seule. |
| `rig_mesh_auto` | Remplace un attachement (région ou maillage) d'un rig par un maillage généré depuis son image : contour par le canal alpha, triangulation, auto-poids (tolérance, espacement, seuil alpha, marge). Le rig est validé avant écriture (une erreur refuse tout), le fichier est réécrit (**hors historique d'annulation**) sauf si l'éditeur de rig a ce rig ouvert : voir plus bas. |
| `rig_create`, `rig_add_image_part`, `rig_delete` | Construction d'un rig sans l'éditeur. `rig_create` : fichier `assets/rigs/<nom>.rig.ron` (jamais écrasé) ; avec `image` (chemin du projet, importée comme asset si besoin) : os `root` aux pieds (centre bas, vers le haut), slot nommé d'après le fichier, maillage tracé depuis l'alpha, image debout sur l'origine ; sans image : rig vide. `add_to_project` déclare l'asset. `rig_add_image_part` : une image de plus (bras, tête…) comme slot au premier plan avec son maillage, sur un os (par défaut la première racine) ; les calques de même taille s'alignent sur l'image existante ; non pondérée jusqu'à `rig_bind`. `rig_delete` : supprime le fichier et sa déclaration, refusé tant qu'une expression joue le rig (destructif, hors annulation). |
| `rig_import_layers` | Importe un fichier à calques (`.psd`, `.kra`, `.ora`) dans un rig (nouveau, ou `rig` existant) : un PNG rogné par calque raster (`images/<rig>/`), un slot et un maillage à la position exacte du calque (origine = centre bas du canevas), ordre de dessin de la pile ; `groups_as_bones` : un os par groupe ; les calques masqués sont gardés comme slots sans image active (`include_hidden`, défaut vrai). Texte, réglages, masques, modes de fusion, PSD compressé ZIP : ignorés avec `import_warnings`. Non pondéré jusqu'à `rig_bind`. |
| `rig_bone_add`, `rig_bone_update`, `rig_bone_reparent`, `rig_bone_delete` | Os d'un rig, par id ou par nom. Unités : pixels, origine aux pieds, Y vers le bas, rotation en degrés dans le sens horaire (-90 = vers le haut). Position `x`, `y`, `rotation` relatives au parent (`space: parent`, défaut) ou dans le rig (`space: world`), ou `head` + `tip` dans le rig (rotation et longueur en découlent, comme le cliquer-glisser de l'éditeur) ; `scale_x`, `scale_y`, `length`, `name` (unique). `rig_bone_update` peut aussi changer de `parent` (`""` = racine ; l'os garde sa place dans le monde). `rig_bone_reparent` refuse une boucle. `rig_bone_delete` retire l'os, ses descendants, les slots portés, l'IK, les pistes de clips et les poids qui s'y rapportent, et liste ce qui est parti. |
| `rig_bind` | Auto-poids de tous les maillages de toutes les skins depuis les os actuels (distance aux segments d'os, lissage, 4 os au plus par sommet, paramètre `falloff`). À rappeler après avoir ajouté des os. |
| `rig_slot_update`, `rig_skin_add`, `rig_skin_duplicate`, `rig_skin_delete`, `rig_ik_set`, `rig_ik_remove` | Slots : nom, attachement affiché au repos (`""` = rien), teinte `[r,g,b,a]` 0..1 ou `#rrggbb[aa]`, os suivi, position dans l'ordre de dessin (0 = derrière). Skins : ajout (vide, retombe sur `default`), duplication, suppression (jamais `default` ni la dernière, refusée si une expression l'utilise). IK : `rig_ik_set` crée ou met à jour la contrainte d'une chaîne (1 os vise la cible ; 2 os `[parent, enfant]` l'atteignent ; `bend_positive`, `mix`), `rig_ik_remove` par index ou par chaîne. |
| `rig_clip_create`, `rig_clip_update`, `rig_clip_delete`, `rig_clip_key_set`, `rig_clip_key_delete` | Clips d'un rig. Création (durée en secondes, `looping` : `once`, `forever`, `ping_pong`, `times` + `times`), changement de durée / boucle (raccourcir sous la dernière clé est refusé), suppression (refusée si une expression joue le clip). `rig_clip_key_set` pose ou remplace une clé à un temps (0 à la durée ; une clé à la même milliseconde est remplacée) ; `track` donne la forme de `value` : `bone_rotate` (degrés, ajoutés au repos), `bone_translate` `[dx,dy]` (ajouté), `bone_scale` `[sx,sy]` (multiplie), `slot_attachment` (id d'attachement ou `null`, par palier), `slot_color`, `deform` (`[[dx,dy],…]`, un par sommet), `draw_order` (slots, derrière → devant, par palier). `ease` : préréglage (`OutQuad`, `InOutSine`, `OutBack`… ou `out_quad`), `{"bezier":[x1,y1,x2,y2]}` ou nom d'une courbe de l'histoire (`set_curve`). `rig_clip_key_delete` retire la clé (la piste vidée disparaît). |
| `rig_add_to_project` | Déclare un fichier de rig comme asset `rig` (annulable) et renvoie son id et ses problèmes. |
| `rig_cut` | Découpe les pixels d'un `polygon` (coordonnées pixels de l'image du slot, origine en haut à gauche, règle pair-impair) en une nouvelle image (rognée, déclarée comme asset) et un nouveau slot avec un maillage tracé, exactement au même endroit, juste devant la source (`below` : derrière), sur `bone` (défaut : l'os du slot). `fill_hole` (défaut vrai) bouche le trou de la source depuis les pixels autour et l'écrit comme **nouvelle** image (l'originale est gardée, le rig pointe vers la copie ; un contour changé retrace le maillage). Pour séparer un personnage plat en calques (mèches de devant, paupières, visage, cheveux de derrière). Renvoie le slot, l'image, sa taille et son décalage dans la source. |
| `rig_duplicate` | Copie un rig sous un nouveau nom (`assets/rigs/<name>.rig.ron`, jamais écrasé) et le déclare comme asset. `copy_pictures` copie aussi ses images dans `images/<name>/` (déclarées comme assets) ; sinon les deux rigs partagent les images. |
| `rig_copy_clips` | Copie des clips (`clips`, défaut : tous) du rig `source` vers `rig`. Os et slots associés par nom (`by_hierarchy` : par position dans l'arbre des os) ; `rescale` adapte les translations au rapport des longueurs d'os ; les clés de déformation sont abandonnées si les maillages n'ont pas le même nombre de sommets ; un nom de clip déjà pris reçoit un numéro. Renvoie `unmatched_bones`, `unmatched_slots`, `unmatched_attachments`, `dropped_deform`. Validé, atomique, une étape d'annulation dans l'éditeur. |
| `rig_recenter` | Mesure les vraies limites des images visibles en pose de repos (skin par défaut, `skin`, ou `all_skins`) et décale os racine et sommets pour que le bas-centre du dessin soit en (0, 0). Les clips restent valides. Renvoie `bounds_before` et `shift`. |
| *(toutes les modifications de rig)* | Chaque appel est **validé** (`validate` d'orbit_anim : une erreur refuse l'appel et rien n'est écrit ; les avertissements, dont le budget web 64 os / 4000 sommets, sont renvoyés dans `warnings`) et **atomique**. La réponse contient un `summary` (os avec id et pose, slots, skins, clips, IK). Si l'éditeur est ouvert avec ce rig dans l'éditeur de rig, la modification passe par son document (annulation Ctrl+Z, vue rafraîchie, fichier sauvegardé) et les lectures voient son état non sauvegardé ; sinon le fichier est réécrit (**hors historique d'annulation**). `rig_sample` accepte `vertices: true` et renvoie dans `meshes` l'étendue et le déplacement maximal des sommets après skinning. |
| `set_expression_rig` | Fait jouer un clip de rig à une expression (rig, skin, clip, `loop_mode`) ; sans `rig`, retour à l'image fixe. Refuse clip ou skin inconnus. Annulable ; un `.rig.ron` non déclaré est d'abord déclaré. |
| `create_scene3d`, `edit_scene3d`, `write_scene3d` | Création (fichier `assets/3d/<nom>.scene3d.ron` + déclaration de l'asset, une entrée d'annulation ; un soleil et une caméra `main` par défaut), édition par opérations ordonnées **tout ou rien** (`add_node`, `remove_node`, `rename_node`, `set_parent`, `set_visible`, `set_transform`, `set_light`, `set_camera`, `set_mesh_materials`, `set_material` avec emplacements de texture, `remove_material`, `set_environment` : couleur ou skybox, ambiance, brouillard) et remplacement complet depuis du RON. Le résultat est validé : une erreur refuse tout et n'écrit rien ; les avertissements sont renvoyés. **Hors historique d'annulation** (le fichier est réécrit) ; une scène ouverte dans l'éditeur 3D doit être rouverte pour voir le changement. |
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

## Mode live : l'éditeur ouvert (M4)

L'éditeur expose le même MCP en **HTTP streamable** sur `127.0.0.1`. L'IA travaille sur **le projet ouvert dans l'éditeur** (le même `Project` que vous) : ses modifications apparaissent en direct à côté des vôtres, chacune est **une entrée d'annulation** que vous pouvez défaire, et elle peut aussi **lancer et regarder la preview** (état, capture d'écran), sélectionner un nœud pour vous le montrer, et construire et exporter le jeu (`build_project`, `export_game`).

### Activer le serveur

Le serveur est **désactivé par défaut** : rien n'écoute tant que vous ne l'avez pas demandé.

1. Ouvrez un projet, puis le panneau **Assistant IA** (icône robot de la barre d'état, menu Outils → Assistant IA (MCP)…, Ctrl+Maj+A ou la palette de commandes ; il est dans la zone du bas, à côté de la console).
2. Choisissez le port (7878 par défaut ; modifiable tant que le serveur est arrêté) puis **Activer le serveur MCP**. L'état passe à « À l'écoute », puis à « N client(s) connecté(s) » quand un client a une session ouverte.
3. Un **jeton d'accès** est généré une fois pour toutes (conservé dans la configuration de l'éditeur) : il est masqué, avec des boutons pour l'afficher, le copier, et en **générer un nouveau** (le serveur redémarre, l'ancien jeton cesse de fonctionner immédiatement).
4. Les boutons **Commande Claude Code** et **.mcp.json** copient une configuration prête à coller, avec l'en-tête d'autorisation.

Pour les scripts et les captures automatisées, l'éditeur accepte `--mcp-port <port>` (démarre le serveur au lancement) et `--mcp-token <jeton>` (jeton imposé).

### Brancher Claude Code

```sh
claude mcp add --transport http orbit http://127.0.0.1:7878/mcp --header "Authorization: Bearer orbit_<jeton>"
```

ou, dans le `.mcp.json` du dépôt de l'IA :

```json
{
  "mcpServers": {
    "orbit": {
      "type": "http",
      "url": "http://127.0.0.1:7878/mcp",
      "headers": { "Authorization": "Bearer orbit_<jeton>" }
    }
  }
}
```

(Le port et le jeton à utiliser sont ceux affichés dans le panneau.) Le point d'entrée est `/mcp`. Sans client MCP, un `curl` suffit pour essayer : `POST /mcp` avec `Authorization: Bearer <jeton>`, `Accept: application/json, text/event-stream` et un message JSON-RPC `initialize` ; le serveur répond avec un en-tête `Mcp-Session-Id` à renvoyer ensuite.

### Comment ça marche

```
client ──HTTP──▶ [fil du serveur : tokio, jeton, rmcp]
                      │ fil MCP (un appel à la fois)
                      ▼
               LiveHost ──▶ file de commandes ──▶ fil de l'éditeur
                      ▲                              │ exécute l'outil sur le Project ouvert
                      └──────── réponse ◀────────────┘ (une entrée d'annulation), puis réveille la boucle
```

* Le serveur tourne sur **son propre thread** ; il ne touche jamais au projet. Chaque appel est posté dans une **file de commandes** (`orbit_mcp::live::CommandQueue`), l'éditeur la vide **sur son fil**, entre deux frames (avant le dessin de l'interface), exécute l'outil par `Project::group` (une entrée d'annulation par appel, tout ou rien) et renvoie le résultat. Le `Waker` de la plateforme réveille la boucle à chaque requête : l'éditeur reste à 0 frame au repos.
* **Les outils sont ceux du mode headless** (`orbit_mcp::host::ToolRunner`, même code, mêmes schémas) ; `ProjectHost` a une implémentation « éditeur » (`LiveHost`, qui ne possède rien et délègue à l'éditeur). Les outils des plugins (`plugin_id.outil`) sont ceux des plugins **chargés dans l'éditeur** (les intégrés y sont).
* **Rien n'est écrit sur le disque tant que vous n'enregistrez pas** (Ctrl+S) : les modifications de l'IA sont des modifications non enregistrées comme les vôtres (sauvegarde automatique de récupération comprise). L'outil `save` écrit tout de suite si l'IA doit lancer ensuite un outil qui lit le disque. Le titre de la fenêtre et la barre d'état le montrent.
* **Conflits** : chaque nœud a une version, partagée entre vous et l'IA. Si vous modifiez un nœud après que l'IA l'a lu, son `edit_dialogue` / `delete_node` / `move_node` avec l'ancien `expected_version` échoue avec `version_conflict` (« le nœud a été modifié depuis votre lecture, peut-être par l'utilisateur dans l'éditeur ; relisez ») et **rien n'est appliqué**. `undo` / `redo` (outils) partagent l'historique de l'éditeur.
* Un appel de l'IA est **annulable en bloc** : bouton « Annuler la dernière action de l'IA » du panneau (n'agit que si cette action est encore la dernière de l'historique : il ne défait jamais vos propres modifications) ou Ctrl+Z.
* **Ce qui n'a pas besoin du fil de l'éditeur** : `initialize`, `ping` et `tools/list` sont répondus directement par le fil du serveur (l'éditeur publie sa liste d'outils dans la file au démarrage du serveur puis toutes les 2 s ; les outils 3D expérimentaux y sont filtrés selon l'interrupteur du moment). Un client obtient donc la liste d'outils même si l'éditeur dort, est en arrière-plan, ou est bloqué.
* **Éditeur occupé** : si une requête qui a besoin du fil de l'éditeur (appel d'outil, ressources) n'est pas prise en charge en **5 s**, elle est retirée de la file (elle ne sera **jamais** exécutée plus tard) et le client reçoit aussitôt une erreur `unavailable` « editor busy: <raison> » au lieu d'attendre en silence. Cas typique : une boîte de dialogue **native** (sélecteur de fichier « Ouvrir un projet », import d'images…) bloque le fil de l'interface tant qu'elle est ouverte. Les boîtes de dialogue de l'éditeur lui-même (récupération, modifications non enregistrées, fichier modifié sur le disque) sont dessinées par egui et **ne bloquent pas** les appels. Une requête abandonnée par le client (annulée, expirée) n'est pas exécutée et ne retarde pas les suivantes.

### Dépannage du mode live

* **`initialize` répond mais `tools/list` ne répond jamais (expiration à 30 s)** : corrigé — la liste vient désormais du fil du serveur. Avec une version plus ancienne de l'éditeur, la liste passait par le fil de l'éditeur et une seule requête bloquée (boîte de dialogue native ouverte, appel précédent en attente) retenait toutes les suivantes jusqu'à 60 s chacune. Mettez l'éditeur à jour, ou fermez la boîte de dialogue ouverte et relancez `/mcp`.
* **« editor busy: … »** : fermez la boîte de dialogue indiquée (ou ramenez l'éditeur au premier plan s'il est figé), puis relancez l'appel ; il n'a pas été exécuté.
* **401** : jeton absent ou faux (en-tête `Authorization: Bearer <jeton>` ; le jeton change si vous le régénérez dans le panneau). **403** : en-tête `Host` autre que `localhost` / `127.0.0.1`.
* **Port occupé** : le panneau affiche l'erreur ; choisissez un autre port (ou `--mcp-port`).

### Outils supplémentaires (live uniquement)

| Outil | Rôle |
|---|---|
| `preview_play` | Lance (ou relance) la preview, depuis le début ou depuis un nœud (`from_node` : l'état réel qu'un joueur aurait à ce nœud). Renvoie l'état de la preview. |
| `preview_step` | Avance de `steps` pas (défaut 1) ou jusqu'au prochain choix (`until_choice`, borné à 500 pas) : termine la machine à écrire, passe à la réplique suivante, saute transitions et pauses. |
| `preview_choose` | Choisit une option du choix affiché (`option` : indice 0-based, texte exact ou début de texte non ambigu). |
| `preview_state` | Lit la preview : statut, nœud courant (id, scène, rang), réplique ou choix affiché, **variables**, **scène déclarative** (décor, personnages en scène avec expression et position, musique). |
| `preview_stop` | Arrête la preview. |
| `preview_screenshot` | **Capture d'écran de la preview** : PNG 1280×720 renvoyé en **contenu image MCP** (base64), après avoir laissé finir la machine à écrire et les transitions ; texte : l'état de la preview. Indépendante de la taille et de la visibilité du panneau Preview. |
| `capture_scene3d` | **Image d'une scène 3D** : rend la scène (`scene` : id, chemin, nom de fichier ou nom sans extension ; `camera?` : nom de caméra, la première par défaut ; `width?` / `height?` : 64 à 2048, 1280×720 par défaut) dans une cible dédiée et renvoie un PNG en contenu image MCP. Lit le fichier tel qu'enregistré sur disque avec les modèles et images de `models/` à côté ; ne demande pas la preview. Une caméra inconnue est une erreur (`not_found`). Sans GPU : `unavailable`. |
| `select_node` | Sélectionne et révèle un nœud dans l'éditeur (le panneau scène défile, l'inspecteur l'affiche). |
| `build_project`, `export_game` | **Destructifs** (`destructiveHint`). Passent par `ProjectHost::export` puis `orbit_export::run_request` (le même code que le panneau Export et `orbit build` / `orbit export`) : `build_project {profile?, out_dir?}` valide, compile et écrit le pak (`build/` par défaut) ; `export_game {target: windows\|linux\|web, profile?, out_dir?}` écrit le zip, le tar.gz ou le dossier web (`export/` par défaut). `out_dir` reste **dans le projet**. Les modifications non enregistrées sont enregistrées d'abord ; la réponse liste les avertissements ; une histoire invalide, un asset manquant ou un template de player absent donnent une erreur lisible (`invalid_arguments` pour une cible, un profil ou un dossier faux). |

Une preview qui ne peut pas démarrer (histoire invalide) renvoie l'erreur de validation. `preview_step` / `preview_choose` / `preview_screenshot` sans preview en cours disent d'appeler `preview_play` d'abord.

**Limite connue (3D, mode headless)** : le serveur headless (`orbit mcp`) n'a pas de moteur de rendu : il lit et édite la structure des scènes 3D mais ne peut pas les photographier ; `capture_scene3d` n'existe qu'en mode live (éditeur avec GPU). La capture ne reflète que le fichier enregistré : une scène ouverte et modifiée dans l'éditeur 3D sans l'avoir enregistrée n'est pas vue.

**[edge] Éditeur sans GPU** : les outils `preview_*` et `capture_scene3d` répondent `unavailable: the preview is not available: this editor has no GPU…` ; **tous les autres outils fonctionnent** (lecture, édition, `validate`, `simulate`, `select_node`...). C'est le cas de `EditorCore` sans fenêtre (testé). La fenêtre de l'éditeur elle-même exige un GPU : un éditeur sans GPU « en CI » est le cœur d'éditeur des tests, pas l'exécutable.

### Le panneau « Assistant IA »

* **État** : arrêté / à l'écoute / client(s) connecté(s) (un client est « connecté » tant qu'il a une session ouverte et a écrit dans les 90 dernières secondes), adresse, port, jeton (masqué), requêtes refusées.
* **Journal en direct** de ce que fait l'IA : outil, arguments résumés, résultat, durée, « annulable », et un lien **« révéler »** (sélectionne le nœud touché dans l'éditeur). Les erreurs y sont en rouge. Le même journal est écrit dans `<projet>/.orbit/mcp.log`.
* **Indicateur discret** quand l'IA modifie le projet : « L'IA modifie le projet » dans le panneau et « IA : modification en cours » dans la barre d'état (l'icône robot reste grise serveur arrêté, verte serveur actif).
* **Annuler la dernière action de l'IA** ; vider le journal.

### Sécurité du mode live

* **Désactivé par défaut**, activation explicite dans le panneau ; l'état « activé » n'est pas mémorisé (au prochain lancement, il faut le réactiver).
* **Écoute uniquement sur `127.0.0.1`** (jamais `0.0.0.0`) ; une connexion qui ne vient pas de la boucle locale est refusée (403).
* **Jeton obligatoire** : toute requête sans `Authorization: Bearer <jeton>` valide est refusée (**401**, avec `WWW-Authenticate: Bearer`), jeton absent, mauvais, tronqué ou d'un autre schéma ; comparaison en temps constant ; jeton de 256 bits aléatoires (`getrandom`, préfixe `orbit_`). Le nombre de refus est affiché.
* **Anti-rebond DNS** : l'en-tête `Host` doit être `localhost`, `127.0.0.1` ou `::1` (contrôle de `rmcp`) ; un site web ne peut pas atteindre le serveur par un nom de domaine qui pointerait vers 127.0.0.1 (403).
* Le jeton est conservé **en clair** dans la configuration de l'éditeur (dossier de configuration de l'utilisateur, comme ses projets récents) : traitez-le comme un mot de passe local. Regénérez-le si vous le montrez par accident (capture d'écran, journal de terminal).
* Mêmes protections que le mode headless : périmètre = dossier du projet (`Sandbox`), erreurs structurées, paniques interceptées, tout ou rien par appel. `build_project` / `export_game` sont signalés destructifs.
* Limite : un client **authentifié** a la même puissance que vous sur le projet ouvert (c'est le but) ; le jeton est la seule barrière, d'où le localhost et l'activation explicite.

### Éditeur et `orbit mcp` sur le même projet : le verrou `.orbit/lock`

L'éditeur et `orbit mcp` gardent chacun le projet en mémoire et le réécrivent en entier : ouverts ensemble sur le même dossier, le dernier enregistrement écraserait l'autre **sans rien dire**. Chaque côté pose donc un **fichier de verrou** `<projet>/.orbit/lock` (propriétaire, pid, battement de cœur réécrit toutes les 3 s ; un verrou sans battement depuis 12 s est un reste de plantage et est ignoré) et **avertit dans les deux sens** :

* `orbit mcp` sur un projet que l'éditeur tient : **refus**, avec le message d'utiliser plutôt le MCP live de l'éditeur (panneau Assistant IA), de fermer l'autre côté, ou de passer `--force` pour passer outre (avertissement sur la sortie d'erreur, et le verrou est repris).
* L'éditeur qui ouvre un projet tenu par un `orbit mcp` : avertissement dans la console, un message d'attention et un bandeau dans le panneau Assistant IA (l'éditeur ne prend pas le verrou : l'autre côté le garde). Si un `orbit mcp --force` reprend ensuite le verrou d'un projet déjà ouvert dans l'éditeur, l'éditeur le voit au battement suivant et avertit de même.
* Un fichier d'histoire ouvert seul (sans `orbit.project.ron`) n'a pas de dossier de projet : l'éditeur n'y crée pas de `.orbit/lock` (mais en lit un s'il existe).
* Le verrou est **consultatif** : il ne bloque pas un éditeur de texte ni `git`. Le watcher de l'éditeur détecte aussi toute modification du fichier d'histoire par un autre outil (question « recharger ? »).

## Sécurité (mode headless)

* **Périmètre = le dossier du projet.** Tout chemin reçu passe par `Sandbox::resolve` : chemins vides ou contenant NUL refusés, tout composant `..` refusé, chemins absolus acceptés seulement s'ils tombent dans le projet, **liens symboliques et jonctions résolus** (un lien qui sort du dossier, ou un lien pendant, est refusé), vérification sur le chemin canonique. Aucune opération ne lit ni n'écrit hors du dossier (`import_asset` ne copie qu'un fichier déjà dans le projet ; le nom de destination est assaini).
* **Opérations destructrices signalées** (`destructiveHint`) : suppression de scène et de nœud. Toutes les modifications sont annulables (`undo`).
* **Journal des actions** : `<projet>/.orbit/mcp.log`, une ligne par appel (horodatage Unix en ms, outil, succès ou erreur, arguments et résultat tronqués). Les erreurs y sont aussi.
* **Aucune panique sur entrée invalide** : arguments mal typés, ids invalides, index géants, JSON absurde → erreur structurée, projet inchangé (test de robustesse + interception des paniques).
* Le serveur écoute sur l'entrée standard du processus que le client a lancé : il n'ouvre aucun port réseau (le mode live HTTP de l'éditeur est décrit plus haut : 127.0.0.1, désactivé par défaut, jeton).
* Limite de taille d'import : 256 Mio par fichier.

## Dettes et limites (M3f)

* Pas de surveillance des fichiers : si le projet est modifié sur disque pendant que le serveur tourne, le serveur n'en sait rien et écrasera à la prochaine sauvegarde. Contre l'éditeur ouvert en même temps, le verrou `.orbit/lock` (voir plus haut) refuse le démarrage (`--force` pour passer outre) ; le mode live donne un seul propriétaire du projet.
* Les plugins intégrés de l'éditeur (`orbit_editor`) ne sont pas chargés par `orbit mcp` : seuls les outils du cœur sont exposés tant qu'aucun plugin n'est installé (`HeadlessHost::install_plugin` existe pour les hôtes et les tests).
* Les assets sont enregistrés et copiés mais leur contenu n'est pas inspecté (un faux PNG est accepté ; `validate` ne vérifie que l'existence du fichier).
* Pas de suppression de personnage, de variable ni d'asset, ni d'édition des nœuds autres que les répliques (supprimer puis recréer) ; pas d'outil de build / export, de preview ni de capture d'écran en headless (ils n'existent qu'en mode live).
* Les libellés d'undo des commandes d'`orbit_project` sont en français ; ceux des appels MCP sont « MCP : <outil> ».

## Dettes et limites du mode live (M4)

* **Export** : branché sur `orbit_export` (`orbit_editor::live::export`). L'export tourne sur le fil de l'éditeur : la fenêtre est occupée (quelques secondes, plus avec la compression brotli du web) et la progression n'est pas montrée ; le panneau Export, lui, travaille sur un fil à part. Le serveur headless n'expose pas ces outils (`orbit export` en ligne de commande).
* **File « commune » avec l'UI** : l'éditeur vide la file de commandes du MCP et ses propres requêtes (`EditorRequest`) sur le même fil, l'une après l'autre (même sérialisation, mêmes versions de nœud), mais l'UI appelle encore `Project` directement : une file unique de `Command` typées pour l'UI reste à faire.
* **`validate` sur une histoire ouverte seule** (sans dossier de projet) : la vérification des fichiers d'assets part du dossier de l'histoire, pas du dossier d'assets que l'éditeur retrouve plus haut (la démo en montre des « fichier absent » factices). Les projets avec manifeste ne sont pas concernés.
* **Une seule preview** : `preview_*` pilote la preview de l'éditeur (celle que vous voyez) ; pas de preview concurrente. `preview_step` / `preview_choose` agissent sur la partie en cours ; après une modification de l'histoire, la preview recharge à chaud comme pour vous.
* **Éditeur sans GPU** : seul le cœur d'éditeur (tests) tourne sans GPU ; l'exécutable à fenêtre exige un GPU (comme le player). Les erreurs `unavailable` sont testées sur le cœur.
* **Pas de notifications serveur → client** (resources mises à jour, journal) : le client relit ; les sessions HTTP ne sont pas reprises après un redémarrage du serveur (le client se réinitialise).
* Le jeton est en clair dans la configuration utilisateur (voir Sécurité) ; pas de jetons à portée limitée (lecture seule) ni d'expiration.
