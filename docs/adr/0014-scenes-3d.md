# ADR 0014 : Renderer 3D et scènes 3D

Statut : proposé (M6-00, à relire avant M6-01).

## Contexte

Le M6 ajoute des décors et scènes 3D à un moteur de visual novel 2D (ADR 0001 : wgpu + winit maison ; ADR 0002 : graphe de scène maison). Contraintes : le web (WebGL2 via wgpu) est une cible de premier rang, l'auteur est no-code, le PC de développement est limité (temps de build), et la compatibilité ascendante (plan §1.3) est impérative : un projet v1→v11 doit s'ouvrir, se jouer et s'exporter sans changement de rendu.

Le cas d'usage dominant n'est **pas** un jeu 3D libre mais : un décor 3D vu depuis une ou plusieurs caméras fixes (ou un mouvement de caméra scripté), avec les personnages 2D et la boîte de dialogue par-dessus. Le design en découle.

## Options examinées

### Pipeline de rendu

1. **Forward PBR metallic-roughness, une passe** (retenu). Fonctionne identiquement en WebGL2 et natif ; MSAA natif bon marché ; transparence simple. Limite : coût proportionnel à lumières × objets, acceptable pour des décors de VN (dizaines d'objets, ≤ 8 lumières).
2. **Deferred.** Écarté : MRT + formats flottants mal supportés en WebGL2, MSAA perdu, transparence en passe séparée de toute façon. Gain nul sur des scènes à peu de lumières.
3. **Forward+ / clustered.** Écarté pour l'instant : nécessite storage buffers / compute absents de WebGL2. Possible plus tard en profil desktop seul, sans changer le format.
4. **Intégrer un moteur existant (Bevy, three-d, rend3).** Écarté : contredit ADR 0001/0002, double la pile (ECS, assets), alourdit fortement le build et le wasm. `three-d` est le plus proche mais imposerait sa boucle et ses types.

### Composition 2D / 3D

1. **3D rendue dans une render target, composée sous la pile 2D** (retenu, plan §0.6). Le stage 2D existant (`orbit_runtime/src/stage.rs`) gagne une couche « fond 3D » : rien ne change pour les projets sans 3D. La résolution de la cible peut différer de la fenêtre (échelle de rendu par profil).
2. **2D et 3D dans un même graphe, sprites en quads 3D.** Écarté pour le cas général : tri, échelle en pixels et tests golden 2D existants seraient perturbés. Gardé en option ultérieure (« billboard » de personnage dans le décor) sans impact sur ce format.

### Données de scène

1. **Format propre `.scene3d.ron` (IR textuel) qui référence des assets glTF importés** (retenu). Diffable, migrable comme le reste (ADR 0004), éditable par commandes `orbit_project` et MCP.
2. **glTF comme format de scène source.** Écarté : l'auteur veut placer caméras fixes, lumières et ajuster des matériaux sans retourner dans Blender ; réécrire du glTF à chaque édition est fragile et non diffable.

## Décisions

### 1. Crates

* `orbit_scene` : données 3D **sans GPU**, compilable wasm (règle §1.2) : `Scene3d { id, nodes, environment, cameras }`, `Node3d { id: Ulid, name, parent, transform (translation, rotation quat, scale), kind }`, `Node3dKind::{Empty, Mesh { mesh: AssetRef, materials: Vec<MaterialRef> }, Light(Light), Camera(CameraDef)}`. Validation (références, cycles, quaternions normalisés, NaN) dans la crate.
* `orbit_render3d` : rendu wgpu (pipeline PBR, ombres, skybox, post-process), dépend d'`orbit_scene`. `orbit_export` / `orbit_pak` / `orbit_mcp` ne dépendent que d'`orbit_scene` (même logique qu'`orbit_ui_data`, UI-01).
* Feature cargo `3d` sur `orbit_runtime` / `orbit_player` (activée par défaut dans l'éditeur et le player desktop ; à l'export web, activée seulement si le projet contient une scène 3D, pour ne pas grossir le wasm des projets 2D).

### 2. Matériaux et rendu

* Matériau **PBR metallic-roughness** aligné sur glTF : base color (+ texture), metallic, roughness (+ texture ORM), normal map, émissif, occlusion, `alpha_mode` (Opaque / Mask / Blend), double face. Un seul shader WGSL « uber » avec définitions de préprocesseur (variantes compilées et mises en cache par clé de features), pas de graphe de shaders.
* Matériau **Unlit** en plus (décors peints, style anime) : cas fréquent en VN, coût minimal.
* Lumières : directionnelle, ponctuelle, spot. **Ombres** : shadow map directionnelle (cascades : 1 en web, jusqu'à 3 desktop) + PCF ; une seule lumière projette des ombres en web, ombres de spots en desktop seulement.
* **Environnement** : couleur unie, skybox cubemap ou équirectangulaire (convertie à l'import). IBL simplifiée : irradiance en harmoniques sphériques (9 coefficients, calculées à l'import, aucune passe GPU au runtime) + cubemap préfiltrée en desktop. Brouillard linéaire/exponentiel.
* Post-process par profil : tonemapping ACES (toujours), exposition, bloom (desktop), vignette, FXAA si pas de MSAA. Espace linéaire HDR (`Rgba16Float`), avec repli `Rgba8UnormSrgb` + tonemapping dans le shader si le format n'est pas rendu en WebGL2 (détection à l'init).

### 3. Profils de qualité

| | Web (WebGL2) | Desktop bas | Desktop haut |
|---|---|---|---|
| Lumières dynamiques | ≤ 4 | ≤ 8 | ≤ 16 |
| Lumières à ombre | 1 (directionnelle) | 1 | 4 |
| Shadow map | 1024, 1 cascade | 2048, 2 | 2048, 3 |
| IBL | SH seules | SH + préfiltrée | idem |
| Bloom | non | oui | oui |
| Textures | KTX2 (Basis UASTC/ETC1S) | PNG/JPEG ou KTX2 | idem |

Le profil web est **conçu et testé en premier** (risque §10). Les limites sont des données (`QualityProfile`) : une scène qui les dépasse ne casse pas ; les lumières excédentaires sont ignorées par ordre d'importance et la validation émet un **avertissement** (M6-12), jamais une erreur bloquante.

### 4. Import glTF

* Crate `gltf` (import `.gltf` / `.glb`). L'import copie les assets dans le projet (`assets/3d/<nom>/`) et produit des assets internes : maillages (positions, normales, UV0, tangentes générées par `mikktspace` si absentes, indices u32/u16), matériaux, textures. La hiérarchie glTF devient des `Node3d` d'un **préfab** réutilisable.
* Le fichier source est conservé et ré-importable : les ULID des nœuds sont mappés par chemin de nom glTF pour préserver les surcharges de l'auteur (matériau modifié, nœud masqué) après ré-import.
* Ignorés avec avertissement : animations, skins, morph targets, caméras/lumières KHR (proposées en import optionnel). Les animations glTF seront prises par `orbit_anim` plus tard (après M5b), sans changer ce format.
* Extensions supportées d'emblée : `KHR_materials_emissive_strength`, `KHR_texture_transform`, `KHR_materials_unlit`, `KHR_texture_basisu`.

### 5. Éditeur 3D (no-code, §1.4)

* Panneau **Scène 3D** : viewport (`orbit_render3d` dans une texture egui), grille, caméra d'édition orbit/fly, sélection par picking (ID buffer en natif, rayon/AABB en repli).
* **Gizmos** translate / rotate / scale, espace local/monde, magnétisme. `transform-gizmo-egui` est évalué en M6-09 ; repli : gizmo maison (le modèle de manipulation est pur et testé, seul le dessin dépend de la crate).
* Hiérarchie (glisser pour re-parenter), inspecteur (transform, lumière, caméra, matériau). Toute édition passe par des commandes `orbit_project` (undo/redo, MCP M6-14).
* **Éditeur de matériaux** avec aperçu sphère/cube sous la lumière d'environnement de la scène ; les matériaux importés sont copiés en surcharge plutôt que modifiés en place.
* **Caméras fixes** : objet `Camera` nommé (perspective, FOV, near/far) ; bouton « placer depuis la vue actuelle » ; aperçu dans un petit cadre au format du jeu. C'est l'unité qu'utilise l'histoire.

### 6. Histoire et runtime

* Nouvelle commande de scène dans l'IR : `ShowScene3d { scene: AssetRef, camera: CameraId, transition }` et `Camera3d { camera, transition (cut / fondu / interpolation avec courbe M5a) }`. `HideScene3d` revient au fond 2D. Dans l'éditeur de graphe, ce sont des choix de liste (scène, caméra), pas de saisie libre.
* **Personnages 2D et dialogue par-dessus la 3D** : la couche 3D remplace le « background » du stage ; sprites, textbox, UI data-driven restent inchangés au-dessus. Un fond 2D et une scène 3D ne coexistent pas sur la même couche (la commande 3D masque le fond, et inversement), ce qui garde la sémantique de `ShowBackground` intacte.
* **État déclaratif** (rollback, saves, skip) : le snapshot VM ne stocke que `{ scene, camera, transition en cours (clip + temps local) }`, jamais d'état GPU. Recharger une save reconstruit la vue. Les mouvements de caméra réutilisent la timeline d'`orbit_anim` : rollback au milieu correct par construction.

### 7. IR, schéma v12, pak

* **Schéma v12** (plan §9 : v12 = M6). Ajouts **uniquement additifs** : nouvelles variantes de commande et nouvelle section optionnelle `scenes3d` du manifeste. Migration v11 → v12 : changement de numéro seul. Aucune variante existante ne change de forme.
* Les `.scene3d.ron` ont leur propre `scene3d_version` (comme `ui_version`), avec leur migrateur textuel, pour faire évoluer la 3D sans bumper l'histoire.
* `GamePack` : nouvelle version (N+1) ajoutant une section optionnelle `scenes3d` (RON) et des blobs binaires de maillages (format interne aligné, `bytemuck`, versionné) ; décodage des versions précédentes conservé (struct dédiée par version, comme v1→v2 dans ADR 0011). Textures : KTX2 pour le web (transcodage à l'export via `basis-universal`, en cache), originales en desktop.
* Saves : le snapshot gagne un champ optionnel `stage3d` (`#[serde(default)]`) ; une save sans ce champ se charge (pas de 3D affichée, ce qui est exact pour un jeu antérieur).

### 8. Compatibilité ascendante

* Un projet ≤ v11 n'a aucune scène 3D : migration triviale, rendu identique (le compositeur ne crée la render target 3D qu'à la première `ShowScene3d`). Test golden : une scène 2D rend au pixel près avec et sans la feature `3d`.
* Fixture figée `examples/fixtures/compat/v11/` + test « charge, valide, se joue, save/load » ; save v11 rechargée en v12.
* Un pak antérieur se lit sans la section 3D. Un runtime **plus ancien** ne lit pas un pak v12 contenant de la 3D : refus propre par numéro de version (déjà le comportement de `orbit_pak`).

## Conséquences

* Le plan M6 est conservé ; précisions : M6-01 inclut `scene3d_version` et la validation ; M6-02 inclut la préservation des surcharges au ré-import ; M6-11 porte les commandes `ShowScene3d` / `Camera3d` / `HideScene3d` (schéma v12) ; M6-13 inclut la détection du format HDR en WebGL2 et l'activation conditionnelle de la feature `3d` à l'export web.
* Coût : deux nouvelles crates et un shader conséquent ; la feature `3d` protège le temps de build des tâches 2D et la taille du wasm des projets 2D.
* Non couvert (backlog) : animations glTF / skinning 3D, personnages 3D, physique, lightmaps cuites, forward+ desktop.
