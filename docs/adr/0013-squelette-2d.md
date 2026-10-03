# ADR 0013 : rig 2D maison (os, maillages, poids, clips)

Statut : proposé (à relire par l'utilisateur avant M5B-01). Dépend de l'ADR 0012 (`orbit_anim`, `Curve`, `Track`, `Clip`).

## Contexte

Les personnages sont des images fixes ou des animations image par image (`Motion::Animation`, schéma v8). L'animation squelettique (respiration, bras qui bougent, clignements, mouvements de tête) donne beaucoup de vie pour peu d'images, mais les outils du marché posent problème :

* **Spine** : runtime sous licence (l'utilisateur final d'un jeu exporté doit avoir une licence Spine) ; **Live2D Cubism** : SDK propriétaire, licence commerciale par chiffre d'affaires. Incompatible avec un moteur libre dont les jeux s'exportent sans condition.
* **DragonBones** : format ouvert mais projet abandonné, format JSON complexe.

Le schéma v8 a réservé `Motion::Rigged` (variante unitaire, ignorée avec l'avertissement `RiggedUnsupported`, l'image « poster » est dessinée). Il faut maintenant la remplir. Contraintes : web (WebGL2), éditeur sans code, rollback/saves (même modèle déclaratif que l'ADR 0012), compatibilité ascendante (§1.3).

## Décisions

### 1. Modèle de données : un asset `.rig.ron`

```rust
struct Rig {
    rig_version: u32,
    bones: Vec<Bone>,          // ordre topologique : parent avant enfant
    slots: Vec<Slot>,          // ordre de dessin par défaut
    skins: Vec<Skin>,          // variantes d'apparence (au moins "default")
    clips: BTreeMap<String, RigClip>,
    ik: Vec<IkConstraint>,     // optionnel, vide par défaut
}
struct Bone { id: BoneId, name, parent: Option<BoneId>, rest: Xform /* translate, rotation, scale, shear=0 */, length }
struct Slot { id, name, bone: BoneId, attachment: Option<AttachmentId>, color }
enum Attachment {
    Region { image: AssetId, xform: Xform },          // image rigide attachée à un os
    Mesh { image: AssetId, vertices: Vec<[f32;2]>, uvs: Vec<[f32;2]>,
           triangles: Vec<[u16;3]>, weights: Vec<[ (BoneId, f32); 4 ]> /* ≤4 os, somme = 1 */ },
}
```

* **Fichier séparé `.rig.ron`** (asset avec `AssetId` stable, comme une image), pas dans l'histoire : un rig est lourd (milliers de sommets), partagé entre expressions et histoires, et édité par un outil dédié. L'histoire n'en garde que la référence. Versionné par `rig_version` avec ses propres étapes de migration textuelle (modèle de `ui_version`), indépendant de `SCHEMA_VERSION`.
* IDs d'os/slots/attachements = ULID courts (ADR 0004) : renommer un os ne casse ni les clips ni les poids.
* Poids : **4 os maximum par sommet**, normalisés à l'enregistrement (validation : somme ≠ 1 ± 1e-3 → normalisé avec avertissement). 4 suffit largement en 2D et correspond au format GPU standard si on passe un jour au skinning GPU.
* `Region` existe parce que beaucoup de rigs VN sont des « pantins découpés » (morceaux rigides) sans maillage : moins cher à produire et à dessiner.
* **Skins** : jeux d'attachements par slot (tenues, mêmes os). Optionnel, mais le coût de format est nul s'il est prévu dès le départ.

### 2. Clips squelettiques = `orbit_anim::Clip`

* `RigClip` réutilise `Clip`/`Track` de l'ADR 0012 avec des cibles propres au rig : `Bone(id).{Rotation, Translate, Scale}`, `Slot(id).{Attachment (discret), Color}`, `Mesh(id).Deform` (décalages de sommets, pour les expressions faciales fines), `DrawOrder` (discret). Même `Curve` par clé, mêmes boucles.
* Une timeline d'histoire (nœud `Timeline`, ADR 0012) peut aussi cibler `Character(id).Bone(name)` pour un geste ponctuel par-dessus le clip courant (additif, comme les autres pistes).
* Évaluation pure : `Rig::pose(clip, t) -> Pose` (transforms locales) → `Pose::world()` (matrices 2×3 monde, parcours dans l'ordre topologique) → `skin(&mesh, &world) -> Vec<Vertex>` (skinning linéaire). Tout dans `orbit_anim`, sans GPU, testé numériquement.
* **Mélange** : changer de clip (`idle` → `talk`) fait un fondu croisé de `blend` secondes (défaut 0,2 s) entre les deux poses ; l'état du fondu est déclaratif `(clip précédent, clip courant, changé_à)`.

### 3. IK : optionnel, réduit

* `IkConstraint { bones: [BoneId; 1 ou 2], target: BoneId, bend_positive: bool, mix: f32 }` : solveur analytique à 1 ou 2 os (loi des cosinus), appliqué après la pose et avant les matrices monde. Pas de FABRIK/chaînes longues : peu utile en VN, coûteux à rendre robuste.
* Livré en dernier dans M5b (sous-tâche de M5B-03/06) et coupable sans impact sur le format : un rig sans `ik` est valide.

### 4. Lien avec l'histoire : `Motion::Rigged` (schéma v11)

* La variante réservée devient `Motion::Rigged(RiggedMotion { rig: AssetId, skin: Option<String>, clip: String, loop_mode: LoopMode })`. `image` reste le poster (miniatures, repli si le rig manque ou ne se charge pas).
* **Migration v10 → v11** : la v8 n'a jamais donné d'effet à `Rigged` (unitaire). La migration textuelle retire `motion: Some(Rigged)` des expressions (rendu identique : le poster était déjà dessiné) et signale le retrait dans le rapport de migration. Le pak : le décodage des `GamePack` antérieurs garde sa struct dédiée où `Rigged` est unitaire et la convertit en `motion: None`.
  * *Alternative écartée* : garder `Rigged` unitaire et ajouter une variante `Skeletal(...)` — laisserait une variante morte à vie dans le format et le schéma JSON du MCP.
* Changement d'expression d'un personnage riggé vers une autre expression **du même rig** : fondu croisé de clips (pas de transition d'image). Vers un autre rig ou une image : transition d'expression habituelle.
* Rollback/saves : l'état est `(expression, clip_started_at)` sur l'horloge de scène de l'ADR 0012 ; rien de nouveau à sauvegarder. Champs nouveaux en `#[serde(default)]`.
* Validation : rig introuvable, clip inconnu, skin inconnue → avertissement, poster dessiné (jamais d'erreur bloquante pour une ressource visuelle, comme les images).

### 5. Skinning et rendu sur WebGL2

| Option | Pour | Contre |
|---|---|---|
| **CPU** : sommets déformés écrits dans un vertex buffer dynamique | identique natif/web, aucune limite d'uniforms, testable sans GPU, simple | coût CPU ∝ sommets ; un VN a 1–4 personnages × 500–3000 sommets : quelques dizaines de µs |
| GPU : matrices d'os en uniform buffer, skinning dans le vertex shader | gratuit côté CPU | WebGL2 : 16 Kio d'UBO garantis (≈ 340 matrices 2×3, OK), mais attributs d'os/poids en plus, un shader de plus à maintenir, golden tests plus délicats |
| GPU par texture de matrices | pas de limite d'os | complexité, échantillonnage de texture en vertex shader parfois lent sur mobile |

**Décision : CPU d'abord**, derrière un trait `Skinner` (`fn skin(&self, rig, pose, out: &mut Vec<MeshVertex>)`) pour pouvoir ajouter un `GpuSkinner` plus tard sans toucher au format. Budget visé (validation, avertissement au-delà) : 64 os et 4000 sommets par rig sur le web.

* Rendu : nouveau pipeline `mesh.wgsl` dans `orbit_render2d` (sommets position+UV+couleur, indexés, même blending premultiplié et mêmes calques que les sprites) ; les `Region` passent par le batcher de sprites existant. Ordre de dessin = slots, à l'intérieur du calque du personnage. Golden tests par readback (M5B-02).
* Atlas : les images d'un rig sont des assets ordinaires ; l'export peut les regrouper avec l'atlas UI-21c si disponible (optimisation, pas requise).

### 6. Éditeur

Un plugin intégré « Rig » (panneau principal + vue dédiée), tout via commandes `orbit_project` (undo/redo), chaînes en/fr :

* **Os** (M5B-03) : création à la souris (clic-glisser = os enfant de l'os sélectionné), gizmos rotation/translation/échelle, hiérarchie en arbre (renommer, reparenter, réordonner), mode « pose » vs « setup » (la pose de repos n'est modifiée qu'en setup).
* **Maillages** (M5B-04) : contour automatique depuis le canal alpha (marching squares + simplification Douglas-Peucker), triangulation de Delaunay contrainte (crate `spade`, pure Rust, compatible wasm mais utilisée seulement dans l'éditeur), ajout/suppression/déplacement de sommets, sommets internes pour mieux plier.
* **Poids** (M5B-05) : auto-poids par distance aux os (inverse distance, 4 plus proches, normalisés), pinceau (rayon, force, ajouter/retirer/lisser), normalisation automatique, visualisation en dégradé par os, test en direct en tournant un os.
* **Clips** (M5B-06) : l'éditeur de timeline de M5A-05 avec les pistes d'os/slots/déformation, scrubbing dans la vue du rig, auto-key en posant le squelette.
* Aperçu dans la vue de scène et l'inspecteur d'expression (choix rig / skin / clip).

### 7. Import (sans Spine ni Live2D)

* **Pas d'import Spine `.skel`/`.json` ni Live2D `.moc3`** dans le moteur : la licence Spine interdit l'usage de ses fichiers sans licence runtime par le développeur final, et `.moc3` est un format propriétaire non documenté. Le risque juridique ne vaut pas le gain.
* **Import PSD/calques** (recommandé, après M5B-08 si le temps le permet) : chaque calque devient une image + un slot avec une `Region` placée à sa position ; c'est le flux de travail habituel des artistes VN (dessin découpé en calques). Crate `psd` (MIT) côté éditeur seulement.
* Format ouvert documenté : le `.rig.ron` et son schéma JSON (`schemars`) sont publics, pour que des scripts externes ou un futur convertisseur communautaire (ex. depuis DragonBones, format ouvert) puissent produire des rigs sans intégrer de code propriétaire.

### 8. MCP

Outils adossés aux commandes `orbit_project` : `rig_create`, `rig_bone_add` / `rig_bone_set`, `rig_slot_add`, `rig_attach_image` (région), `rig_mesh_auto` (contour + triangulation + auto-poids), `rig_clip_key_set`, `rig_sample` (pose monde à un temps donné, pour vérifier sans rendu), `expression_set_rigged`. La peinture fine des poids reste un geste d'éditeur.

## Conséquences

* Aucune dépendance propriétaire : les jeux exportés restent libres de droits côté moteur.
* `orbit_anim` porte toute la logique (pose, IK, skinning) et se teste sans GPU ; le rendu n'ajoute qu'un pipeline mesh.
* Le skinning CPU est un choix réversible (trait) ; si un jeu dépasse le budget, le GPU arrivera sans changement de format.
* Les rigs vivent dans des assets versionnés séparément : un changement de format de rig ne bumpe pas le schéma de l'histoire.
* La migration v11 supprime une valeur qui n'avait jamais d'effet : aucun projet ne change d'apparence.
* L'éditeur de rig est le gros du coût de M5b (maillage et poids en L chacun) ; l'import PSD réduit fortement le travail d'entrée pour les artistes et passe en tâche optionnelle.

## Addendum M5B-07 / M5B-08 : runtime, saves, export, MCP (schéma v14, pak v5)

### Format

* **Schéma v14** (le §4 annonçait v11 : la numérotation a avancé entre-temps). `Motion::Rigged(RiggedMotion { rig: AssetId, skin: Option<String>, clip: String, loop_mode: LoopMode })` remplace la variante unitaire ; `AssetKind::Rig` (ajouté en dernier) désigne un fichier `.rig.ron`, rangé par l'éditeur dans `assets/rigs/`. `Expression::assets()` renvoie le poster puis l'asset rig.
* **Migration v13 → v14** : l'ancien `motion: Some(Rigged)` (ou `motion: Rigged` avec `implicit_some`) est **supprimé du texte avant toute étape de migration** (`drop_unit_rigged`, pour toute version < 14 : les étapes suivantes lisent le texte avec les types courants, qui ne savent plus lire la variante unitaire). L'expression devient une image fixe : le poster était déjà dessiné, **aucun changement visuel**. Fixture figée : `examples/fixtures/compat/v13/story.ron` (avec un `Rigged` unitaire) ; elle est dans la liste de `tests/compat.rs`.
* **Pak (GamePack)** : la disposition de `Motion::Rigged` change (postcard n'est pas auto-descriptif), donc **version 5**, écrite seulement pour un jeu qui a une expression riggée (`GamePack::uses_rigs`, choisie par `to_bytes` / `to_bytes_with_3d`) ; les autres jeux restent en version 3 (ou 4 avec de la 3D), lisibles par les lecteurs plus anciens. Les versions 1 à 4 se lisent avec la disposition de leur époque : `Expression` a un `Deserialize` manuel qui, quand le lecteur déclare un pak ≤ 4 (`orbit_core::with_legacy_motion_layout`, drapeau local au thread), lit `Motion` avec l'ancienne variante unitaire et la convertit en image fixe. Fixture figée : `examples/fixtures/compat/pak_v3_rigged/game.pack` (un `Rigged` unitaire dans un pak v3).
* **Saves** : aucun changement (format 6). Le clip n'est pas sauvé : c'est l'état déclaratif `(expression, horloge de scène)` ; la scène sauvée contient déjà l'expression, l'horloge d'animation repart avec la partie chargée, comme pour les animations image par image.
* Le `.rig.ron` garde son propre `rig_version` (inchangé : 1).

### Conventions de rendu

* Les **unités du rig sont les pixels de l'image poster** de l'expression ; l'origine (0, 0) est l'**ancre bas-centre** du personnage. Le rig est mis à l'échelle comme l'image fixe (hauteur standard du personnage rapportée à la hauteur du poster) puis par l'échelle du `Show` et les effets : échelle uniforme (x) plus un étirement vertical, rotation et teinte des effets, miroir X, miroir Y « en place » (autour du milieu de l'image).
* `loop_mode` de l'expression **remplace** la boucle du clip (`Loop` = en boucle, `Once` = tient la dernière pose, `PingPong`). Les courbes nommées de l'histoire s'appliquent aux pistes du clip.
* **Repli** : tant que le rig ou ses images ne sont pas chargés, ou s'il est illisible, le poster est dessiné (jamais d'erreur bloquante).

### Temps, fondu, mouvement réduit

* La pose est une **fonction pure** de `(expression, clip, loop_mode, horloge)` : `AnimClock` (le temps depuis l'apparition de chaque couple personnage/expression, qui n'avance que tant que le jeu tourne, donc **aussi pendant qu'une réplique s'écrit ou attend un clic** : horloge partagée) ; rien n'est stocké par image. `Game::stage_is_animated` demande des redessins tant qu'une expression riggée est à l'écran.
* **Changement d'expression** d'un personnage qui reste en scène vers une autre expression du **même rig et de la même skin** (`None` = skin `default`) : fondu croisé de 0,2 s (`RIG_BLEND`) entre le clip quitté, qui continue sur son propre temps, et le nouveau, via `RigPose::blend` ; l'horloge mémorise `(expression quittée, début, instant du changement)`. Vers un autre rig, une image fixe ou une nouvelle arrivée : pas de fondu (transition d'expression habituelle).
* **Mouvement réduit** : la première pose du clip est tenue (temps 0), pas de fondu. Les animations image par image gardent leur comportement.
* Rollback / saut / chargement : l'horloge repart, la pose retombe sur celle du début du clip, sans état à restaurer (testé : `AnimClock::at` redonne la même pose qu'une horloge jouée).

### Chargement et export

* `orbit_runtime::rigs::RigLoad` : charge le `.rig.ron` (via `AssetLoader`, dossier, pak ou web), le prépare (`PreparedRig`), puis charge les images qu'il nomme (chemins du projet, avec cache par chemin) ; le tout est un seul `PendingAsset`. Aucun format de pak nouveau pour les assets : le rig et ses images sont des entrées ordinaires.
* Export (`orbit_export::compile`) : pour chaque rig utilisé, les images qu'il dessine **sont ajoutées au pak même si l'histoire ne les déclare pas** ; une image absente bloque l'export (`MissingAssets`), un rig illisible ou un chemin hors projet est un avertissement.

### Validation

* Cœur (`orbit_core::validate`) : l'asset doit exister et être de kind `Rig`, le nom de clip n'est pas vide (`InvalidAnimation`). L'avertissement `RiggedUnsupported` est supprimé.
* Avec les fichiers du projet (`orbit_project::rigs::rig_problems`) : fichier absent ou illisible, clip ou skin inconnus, image absente, budgets web du rig ; ce sont des **avertissements** (le poster est dessiné). Utilisé par `orbit validate --check-files`, le panneau Problèmes de l'éditeur et l'outil MCP `validate`.

### Éditeur et MCP

* Panneau des personnages : un type d'expression « Rig » (rig, skin, clip, boucle) avec vignette animée. Le maillage automatique et les poids (`meshgen`, `weights`) ont été déplacés de l'éditeur vers `orbit_project::rigmesh` pour être partagés avec le MCP (`spade` y est maintenant une dépendance, hors du chemin du moteur et du lecteur).
* Outils MCP : `rig_list`, `rig_read`, `rig_sample`, `rig_mesh_auto`, `set_expression_rig` (voir `docs/mcp.md`). `rig_create` / `rig_bone_*` / `rig_clip_key_set` de la liste du §8 ne sont pas livrés : la construction des os et des clips reste un geste d'éditeur ; un script peut produire le `.rig.ron` (format ouvert) puis `rig_read` / `validate` pour le contrôler.

### Reporté

* **Pistes de timeline d'histoire ciblant `Character(id).Bone(nom)`** (§2) : non livrées. Elles demandent une nouvelle cible `AnimTarget`, un état d'os dans `StageFx` et l'édition des pistes ; le besoin (un geste ponctuel par-dessus le clip) est couvert pour l'instant par un autre clip et le fondu croisé. À reprendre avec un addendum de l'ADR 0012.
* Peau « tenue » (changement de skin au cours d'un clip) : la skin est fixée par l'expression.
* Fixture : `examples/stories/test_rigged.ron` + `examples/fixtures/assets/robo.*` (images générées par `generate_rig_fixture`, joué en lecture réelle par les tests de `orbit_runtime`). Démo : l'expression « sad » d'Alice respire (`assets/demo/alice_sad.rig.ron`, une région sur un os, générée par `generate_demo_rig`).
