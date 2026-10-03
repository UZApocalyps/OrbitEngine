# ADR 0012 : animation déclarative, courbes, effets et timeline

Statut : proposé (à relire par l'utilisateur avant M5A-01).

## Contexte

Le moteur sait afficher des images fixes, des transitions de scène (`orbit_render2d/src/transition.rs`, durée + type, sans courbe) et, depuis le schéma v8, des personnages animés image par image (`Motion::Animation`). Il manque tout ce qui fait vivre une scène de visual novel : déplacer un personnage, secouer l'écran, zoomer, flasher, teinter, enchaîner plusieurs mouvements en parallèle, des particules (pluie, neige) et des fonds en parallax. TODO_LATER demande aussi des courbes d'easing réutilisables.

Contraintes qui pèsent sur la conception :

* **Rollback, skip, seek, sauvegardes** : le joueur revient en arrière, saute, charge une partie au milieu d'une scène. Tout état d'animation qui n'est pas reconstructible à partir de l'IR et d'un petit état VM est un bug en attente (dérive, double application, état perdu après chargement).
* **Web** : WebGL2, wasm, budget de taille du bundle surveillé par la CI (TL-03), flottants identiques natif/wasm.
* **Éditeur d'abord** (plan §1.4) : tout doit être faisable sans code et exposé au MCP via `orbit_project`.
* **Compatibilité ascendante impérative** (plan §1.3) : un projet v1…v8 s'ouvre, se joue et s'exporte à l'identique.
* Le squelette 2D (ADR 0013) devra réutiliser le même modèle de temps et de courbes.

## Décisions

### 1. Une crate `orbit_anim`, sans GPU et sans horloge

* Crate core (règle §1.2 : compilable `wasm32-unknown-unknown`, pas d'`Instant`, pas de `std::fs`). Dépendances : `serde`, `libm` (fonctions transcendantes identiques natif/wasm), `glam` si déjà présent dans le workspace, rien d'autre.
* `orbit_core` dépend d'`orbit_anim` pour les **types de données** (`Curve`, `Track`, `Clip`, `Key`) ; `orbit_anim` ne dépend pas d'`orbit_core` (pas de cycle). Les références à des cibles de l'histoire (personnage, fond, caméra) sont des types d'`orbit_core` qui *enveloppent* les pistes génériques.
* Le temps est toujours passé en argument : `Clip::sample(t) -> Pose`. Aucune fonction d'`orbit_anim` ne « fait avancer » quoi que ce soit.

*Alternatives écartées* : tweens directement dans `orbit_runtime` (impossibles à tester sans GPU, double modèle de temps avec le squelette) ; une crate tierce (`keyframe`, `bevy_tweening`…) : pas de sérialisation stable, pas de Bézier éditable, dépendances lourdes, pas de garantie de déterminisme wasm.

### 2. Courbes : presets nommés + Bézier cubique

```rust
enum Curve {
    Preset(Ease),                 // Linear, InQuad, OutQuad, InOutQuad, InCubic…, InOutSine,
                                  // InBack/OutBack/InOutBack, OutBounce…, OutElastic…, Step
    Bezier { p1: [f32; 2], p2: [f32; 2] }, // CSS cubic-bezier : P0=(0,0), P3=(1,1)
    Named(String),                // référence à Story::curves
}
```

* `Bezier` suit la convention CSS (`x1, y1, x2, y2`, x bornés à [0,1], y libres pour les dépassements) : familière, deux points éditables, inverse en x calculable. Évaluation : Newton (4 itérations) avec repli bisection (précision 1e-6), via `libm`.
* `Preset` couvre les cas courants sans demander de Bézier (bounce et elastic ne sont pas exprimables par une Bézier unique).
* `Curve::eval(t)` : `t` est bornée à [0,1] ; `eval(0) = 0` et `eval(1) = 1` exactement pour toutes les courbes (garanti par construction, testé par propriété) — c'est ce qui rend « skip → état final » exact.
* **Courbes nommées** : `Story::curves: BTreeMap<String, Curve>` (panneau Définitions). Une `Named` inconnue → avertissement de validation, évaluée en `Linear`. Une `Named` ne peut pas pointer vers une autre `Named` (pas de cycle ; erreur de validation).

*Alternatives écartées* : Bézier multi-segments (plus puissant, mais l'éditeur et le format se compliquent pour un gain rare en VN ; les clés de timeline fournissent déjà le multi-segment) ; courbes échantillonnées en table (taille, non éditables).

### 3. Pistes, clés et clips

```rust
struct Key<T> { t: f32 /* s, relatif au clip */, value: T, ease: Curve /* vers la clé suivante */ }
struct Track<T> { keys: Vec<Key<T>> }      // triées par t, t uniques
struct Clip { duration: f32, looping: Loop /* Once | Loop(n) | Forever | PingPong */, tracks: … }
```

* Types interpolables (`trait Lerp`) : `f32`, `Vec2`, couleur RGBA linéaire, angle (chemin le plus court optionnel). Les valeurs discrètes (visible, image, slot de squelette) utilisent un `Track<Discrete>` en escalier.
* `Track::sample(t)` : avant la première clé → première valeur ; après la dernière → dernière valeur ; entre deux clés → `lerp(a, b, ease.eval(u))`.
* Une piste de l'histoire = **(cible, propriété, `Track`)**. Cibles : `Character(id)`, `Background`, `Camera`, `Layer(n)` (parallax), `Screen` (flash/teinte plein écran), plus tard `Bone(rig, os)` (ADR 0013). Propriétés : `Offset`, `Scale`, `Rotation`, `Opacity`, `Tint`, `Shake` (amplitude), `Zoom`, `Pan`.
* Les pistes produisent des **décalages relatifs à l'état de mise en scène** (position déclarée d'un personnage, cadrage par défaut de la caméra), pas des positions absolues : déplacer un personnage dans l'éditeur ne casse pas une timeline existante, et une timeline peut s'appliquer à n'importe quelle position de départ.

### 4. Où vivent les animations : nœud `Timeline` + commandes rapides (sucre)

Arbitrage demandé par M5A-00 :

| Option | Pour | Contre |
|---|---|---|
| A. Pistes **par scène** (une timeline globale) | vision d'ensemble, comme un logiciel vidéo | le VN avance par répliques à durée inconnue (le joueur lit) ; une timeline globale en secondes ne s'aligne pas sur les clics ; rollback = « où en était-on dans la timeline globale » à chaque réplique |
| B. Nœud `Timeline` **dans le graphe** | s'insère entre deux répliques comme n'importe quel nœud ; le temps local démarre à l'entrée du nœud ; rollback naturel par point de retour | pas de vision globale d'une scène entière |
| C. Seulement des commandes simples | trivial | pas d'enchaînements parallèles, pas d'éditeur de timeline |

**Décision : B, plus des commandes rapides compilées en mini-timeline.**

* `NodeKind::Timeline { clip: Clip, tracks: Vec<StoryTrack>, wait: bool }` : `wait = true` bloque l'avancée jusqu'à la fin (clic = skip vers l'état final) ; `wait = false` lance l'animation et passe au nœud suivant (l'animation continue pendant les répliques).
* `NodeKind::Effect(Effect)` avec `Effect::{Move, Shake, Zoom, Flash, Tint}` (paramètres : cible, durée, courbe, intensité / destination, `wait`). Chaque effet a une fonction **pure** `Effect::to_clip() -> (Clip, Vec<StoryTrack>)` : le runtime n'a qu'un seul chemin d'exécution (celui des timelines), l'inspecteur garde un formulaire simple. Un bouton « Convertir en timeline » remplace le nœud par le `Timeline` équivalent (sens unique).
* Shake : pas de bruit aléatoire libre ; décalage = somme de sinus à fréquences fixes multipliée par l'amplitude de la piste (`libm`, déterministe, reproductible au seek).
* `Loop::Forever` n'est autorisé qu'avec `wait = false` (validation : erreur sinon, l'histoire serait bloquée).
* Les animations non bloquantes s'arrêtent à la prochaine `Scene` (changement de scène) ou par un nœud `Effect::Stop { target }`.

*Option A écartée* pour le modèle de données ; l'éditeur de timeline (M5A-05) peut néanmoins afficher les timelines non bloquantes d'une scène sur une même règle, ce qui donne une partie de la vision d'ensemble sans changer le format.

### 5. État d'exécution : déclaratif `(timeline, temps de départ)`

C'est la décision centrale.

* L'état VM ne contient **que** la liste des animations actives : `ActiveAnim { node: NodeId, started_at: f64 }`, où `started_at` est sur l'**horloge de scène** de la VM (temps de jeu écoulé depuis l'entrée dans la scène, avancé par le runtime avec `dt`, gelé en pause et dans les menus).
* Le rendu calcule à chaque image `pose = Σ clip.sample(scene_clock - started_at)` pour chaque animation active, appliquée par-dessus l'état de mise en scène. Rien n'est accumulé d'une image à l'autre : pas de dérive, pas de double application.
* **Skip / clic pendant une timeline bloquante / mode « sauter »** : l'animation est marquée terminée → on échantillonne `t = duration` (exact grâce à `eval(1) = 1`). Une animation `Forever` en mode saut continue à tourner (c'est décoratif).
* **Rollback** : le point de retour d'une réplique contient déjà le snapshot VM ; il contient maintenant `active_anims` et `scene_clock`. Revenir à une réplique restaure les animations non bloquantes qui tournaient à ce moment-là et leur phase. Les timelines bloquantes antérieures sont terminées (état final), jamais rejouées au rollback.
* **Seek** (`orbit_vm/src/seek.rs`, aperçu de l'éditeur) : traverser un nœud `Timeline` ou `Effect` en seek = l'ajouter terminé (ou actif pour `Forever`), donc l'état visuel atteint est identique à une lecture complète.
* **Saves** : `active_anims` et `scene_clock` vont dans le snapshot sauvegardé (champs `#[serde(default)]` : une save d'avant M5a se charge avec une liste vide, ce qui est exact puisqu'aucune animation n'existait). Les clips eux-mêmes ne sont jamais sauvegardés : ils sont relus dans l'IR par `NodeId`. Si l'histoire a été modifiée et que le nœud n'existe plus ou n'est plus une timeline, l'entrée est ignorée silencieusement (avertissement de log) — même politique que pour un nœud de reprise disparu.
* **Réduction des animations** (M7-07) : une préférence force toutes les animations à leur état final immédiat (même chemin que le skip), plus shake et flash désactivés.

*Alternative écartée* : tweens impératifs avec état courant (`position += vitesse·dt`) sérialisé — fragile au rollback, à la reprise après modification de l'histoire et aux différences de cadence.

### 6. Particules et parallax

* **Particules** : `Emitter { preset | params, rate, lifetime, velocity, gravity, spawn_area, sprite, seed }` dans une piste (cible `Layer` ou `Screen`), activé par une timeline ou un effet `Weather`. Simulation CPU **sans état** : la particule *i* naît à `i / rate`, ses paramètres viennent d'un hash `(seed, i)` (pas de `rand`, pas d'état RNG du VM consommé), sa position à l'instant *t* est analytique (`p0 + v·a + ½·g·a²`, `a` = âge). On n'évalue que les particules vivantes (`i` dans une fenêtre). Rollback, saves et seek ne stockent donc rien de plus que `started_at`. Rendu : quads instanciés dans le batcher existant d'`orbit_render2d`. Plafond par émetteur (web : 512 particules, desktop : 4096 ; validation avertit au-delà).
  * Raison : le plan proposait « RNG seedé du VM » ; un hash par particule est strictement mieux (pas de consommation d'état VM, accès aléatoire en temps, identique au seek). La tâche M5A-08 est mise à jour en ce sens.
* **Parallax** : un fond peut être une pile `layers: Vec<BgLayer { image, depth: f32, repeat_x }>` (le champ `image` existant reste le cas d'une seule couche). Le décalage d'une couche = `pan_caméra × (1 - depth)`. La caméra 2D (`orbit_render2d/src/camera.rs`) gagne `pan` et `zoom` pilotés par les pistes `Camera`. Aucun état propre : tout vient des pistes.

### 7. Représentation IR (schéma v9) et formats

* Un seul bump pour tout M5a : **v9** (sérialisé dans l'ordre M5A-02 → 03 → 04, plus UI-21c et M5A-06 si prêts avant publication). Champs ajoutés :
  * `Story::curves` (`BTreeMap`, omis si vide) ;
  * `NodeKind::Timeline`, `NodeKind::Effect` (nouvelles variantes) ;
  * `Background::layers` (omis si vide) ;
  * `Transition::curve: Option<Curve>` (omis si `None` ; `None` = comportement actuel exact).
* Tous les ajouts sont **additifs** et « strip defaults » : un fichier v8 ne contient aucun de ces champs ; la migration v8 → v9 est un simple changement de numéro. Un projet v8 sauvegardé en v9 sans utiliser d'animation reste textuellement identique au numéro de version près.
* Les durées sont en secondes `f32` ; les temps d'horloge de scène en `f64` (une scène peut durer des heures en mode auto).
* **Pak** (`orbit_pak`) : `GamePack` passe à la version suivante ; les variantes et champs nouveaux sont toujours écrits en binaire (postcard n'est pas auto-descriptif) ; le décodage des versions précédentes est conservé via des structs dédiées, comme pour la v2 (ADR 0011).
* **Saves** (`orbit_save`) : champs `active_anims`, `scene_clock` en `#[serde(default)]` ; anciennes saves chargées sans perte. Une save v9 ouverte par un moteur plus ancien n'est pas un cas supporté (pas de compatibilité descendante, comme jusqu'ici).
* **Compatibilité (§1.3)** : fixture figée `examples/fixtures/compat/v8/` + test « v8 → charge, valide, se joue jusqu'au bout, sauvegarde/recharge, s'exporte » ; save v8 rechargée en v9.

### 8. Intégration runtime

* `orbit_vm` : gère `active_anims`, `scene_clock`, l'entrée dans un nœud `Timeline` / `Effect` (avance immédiate si `wait = false`, attente sinon), le skip et l'arrêt au changement de scène. La VM ne connaît pas les valeurs animées, seulement les temps.
* `orbit_runtime/src/stage.rs` : à chaque image, échantillonne les clips actifs et compose les décalages sur la mise en scène avant de construire la liste de dessin ; caméra, parallax, flash/teinte plein écran (quad coloré) et particules en découlent.
* Les transitions de scène existantes acceptent une `Curve` optionnelle.
* L'éditeur (runtime embarqué et aperçu) utilise exactement le même chemin.

### 9. Éditeur

* **`CurveEditor`** (`orbit_editor_api`, réutilisable) : grille [0,1], deux poignées Bézier, liste de presets avec vignettes, aperçu animé (une pastille qui se déplace), copier/coller en notation CSS `cubic-bezier(...)`. Utilisé dans le panneau Définitions (courbes nommées), l'inspecteur d'effet, les clés de timeline et les transitions.
* **Inspecteur d'effet** : cible, durée, courbe, intensité, « attendre la fin », bouton « Aperçu » et « Convertir en timeline ».
* **Éditeur de timeline** (panneau dock, M5A-05) : règle temporelle, scrubbing avec aperçu dans la vue de scène (seek de l'aperçu au nœud + temps local), une ligne par piste (cible × propriété), clés glissables, multi-sélection, copier/coller, interpolation par clé avec `CurveEditor`, mode « auto-key » (déplacer un personnage dans la vue de scène crée une clé). Chaque action passe par une commande `orbit_project` (undo/redo).
* Particules : inspecteur avec presets (pluie, neige, pétales, poussière) et aperçu en direct ; parallax : couches réordonnables avec profondeur, aperçu en panoramique.
* Toutes les chaînes en `en` et `fr` ; erreurs dans le panneau Problèmes (`Forever` bloquant, courbe nommée inconnue, cible absente de la scène, clés non triées ou hors durée, plafond de particules).

### 10. Web

* Pas de shader nouveau pour M5a hors particules (quads instanciés dans le pipeline sprite existant) : budget WebGL2 inchangé.
* `orbit_anim` est petit et sans dépendance lourde ; CI : taille du bundle wasm comparée au budget (M5A-10), échec si dépassement.
* Déterminisme : `libm` pour sin/cos/pow ; tests de propriété exécutés en natif et un test d'égalité bit à bit sur un échantillon en wasm (`wasm-bindgen-test`) pour courbes, shake et particules.

### 11. MCP

* Outils dans `orbit_mcp/src/tools.rs`, adossés aux commandes `orbit_project` (jamais d'écriture directe) : `curve_set` / `curve_remove` / `curve_list` ; `effect_add` (type, cible, paramètres) ; `timeline_add`, `timeline_track_add`, `timeline_key_set` / `timeline_key_remove` ; `timeline_sample` (renvoie l'état des propriétés à un temps donné, pour qu'un agent vérifie son travail sans rendu) ; plus la capture de preview existante au temps choisi.
* Schéma JSON généré (`schemars`) pour `Curve`, `Effect`, `Clip`.

## Conséquences

* Un seul modèle de temps (déclaratif) pour effets, timelines, particules, parallax et, demain, squelette : rollback, skip, seek et saves n'ont qu'un cas à traiter et se testent sans GPU.
* Les effets simples restent simples à éditer et à écrire en RON ; la puissance de la timeline n'est payée que par ceux qui l'utilisent.
* Coût : les pistes sont relatives à la mise en scène, ce qui demande à `stage.rs` une étape de composition claire (base + Σ décalages). Une propriété animée par deux timelines simultanées s'additionne (décalages) ou se multiplie (échelle, opacité) ; c'est documenté, pas d'arbitrage par priorité.
* Coût : les animations non bloquantes vivent au-delà de leur nœud ; l'éditeur doit les montrer (indicateur sur le nœud, liste dans la timeline de scène) pour éviter les surprises.
* Modification du plan : M5A-08 utilise un hash `(seed, index)` au lieu d'un RNG VM.

## Addendum M5A-03 / M5A-04 : effets et nœud Timeline (schéma v9, parties 2/3 et 3/3)

Réalisation des §4, §5 et §7. Précisions et écarts par rapport au texte ci-dessus :

* **Modèle de données** (`orbit_core::effects`, ré-exporté par `orbit_core`) : `NodeKind::Effect(Effect)` et `NodeKind::Timeline { duration, looping, wait, tracks }`, ajoutés **à la fin** de `NodeKind`. `StoryTrack { target: AnimTarget, prop: AnimProp, data: TrackData }` (le `Clip` d'`orbit_anim` fournit `Loop`, `TrackData`, l'échantillonnage). Cibles : `Character`, `Background`, `Camera`, `Screen`. Propriétés : `Offset`, `Scale`, `Rotation`, `Opacity`, `Tint`, `Shake`, `Zoom`, `Pan`. `AnimProp::applies_to` fixe les couples cible x propriété valides, `AnimProp::accepts` le type de valeur.
* **Sémantique additive** : les valeurs sont relatives à la mise en scène. Décalages, rotations, secousses et pans **s'additionnent** ; échelles, opacités et zooms **se multiplient** ; les teintes sont **composées dans l'ordre de démarrage** (source over). `Move` est donc un déplacement *de* `(dx, dy)` (le ramener = un `Move` opposé), pas une destination absolue ; une animation terminée garde sa valeur finale jusqu'au changement de scène. Un `Flash` terminé (alpha 0) ne retire pas une `Tint` plus ancienne.
* **Effets** : `Move`, `Shake`, `Zoom` (caméra), `Flash` (écran), `Tint` (alpha = `strength`, `0` efface). `Effect::tracks()` est pure et unique : le runtime n'exécute que des pistes ; `to_timeline()` donne le nœud `Timeline` équivalent (bouton « Convertir en chronologie »). Pas de `Effect::Stop` pour l'instant (une animation non bloquante s'arrête au changement de scène).
* **Secousse** : `shake_wave(t)` = somme de sinus à fréquences fixes (`libm`), amplitude de la piste `Shake` ; exactement nulle quand l'amplitude vaut 0.
* **État VM** : `VmState { active_anims: Vec<ActiveAnim { node, started_at }>, scene_clock: f64 }`. Le temps est avancé par l'hôte (`Vm::advance_clock(dt)`, `Director::update` ; gelé en pause puisque `Shell::update` n'appelle pas la partie jeu). Une animation qui revient sur le même nœud remplace l'entrée précédente ; plafond `MAX_ACTIVE_ANIMS = 256`. Le **changement de scène** (`JumpScene`, `goto_scene`) vide la liste et remet l'horloge à 0 ; `Hide` retire les animations qui visent le personnage ; `reconcile` (restore) écarte silencieusement les animations dont le nœud a disparu.
* **Nœud bloquant** : `StepResult::WaitAnimation { node, duration }` (nouvelle variante, pas de réutilisation de `WaitTransition`, pour que le `Director` sache quoi finir). Le `Director` l'affiche comme `Flow::Transition` (un clic ou le mode « sauter » l'achève) et appelle `Vm::complete_anim(node)` : l'animation est placée à `total + 1 ms`, donc l'état final est exact quelle que soit la précision flottante. `Vm::complete_anims()` achève tout (les animations infinies sont laissées telles quelles).
* **Seek** (`seek.rs`) : après chaque pas, `complete_anims()` ; l'état atteint (animations finies, infinies à la phase 0) équivaut à une lecture complète.
* **Rendu** (`orbit_runtime/src/stage.rs`, `stage_scene`) : `Vm::stage_fx()` compose les animations en `StageFx` ; `animate_placement` / `animate_background` appliquent caméra (zoom autour du centre puis pan), décalage, échelle, rotation, opacité et teinte multiplicative des sprites ; `screen_overlay` dessine un quad plein écran entre les personnages et l'UI (`Layer(15)`).
* **Formats** : sauvegardes **format 3** (`orbit_save::FORMAT_VERSION`) car le snapshot VM est en postcard ; le décodeur v2 (`legacy_v2.rs`) et v1 sont conservés, fixture figée `examples/fixtures/compat/save_v2/save.orbs` ; `story_hash` : tags 19 (`Effect`) et 20 (`Timeline`). **Pak** : `PACK_VERSION` reste 3 (le schéma v9 n'est pas publié ; les nouvelles variantes sont ajoutées en fin d'énumération, donc un pak 3 existant se relit) ; `Key` n'écrit `ease` linéaire que dans les formats lisibles (écriture à la main, comme `Character`), sinon le postcard désalignerait. Fixture figée du schéma `examples/fixtures/compat/v9/story.ron` (tous les effets, deux timelines, easing de transition).
* **DSL texte** : `move <cible> by (dx, dy) over <s> [ease <courbe>] [wait]`, `shake`, `zoom`, `flash`, `tint`, `timeline over <s> [loop <n> | forever | pingpong] [wait]` suivi de lignes `track <cible> <prop> <t> <valeur> [ease <courbe>], …` (sans id, comme les options de `menu`). `<courbe>` = nom de preset, `bezier(x1, y1, x2, y2)` ou nom entre guillemets (courbe de l'histoire). L'easing des transitions est maintenant écrit et lu : `with dissolve(0.5, ease OutQuad)`, `with fade(1.0, "#000000", ease "snappy")` (il était perdu par le DSL). Nouveaux mots réservés : `move shake zoom flash tint timeline track by over ease forever pingpong camera screen background bezier rgba` (un nom identique est écrit entre guillemets).
* **MCP** : `add_command type=effect` (champ `effect` : move / shake / zoom / flash / tint) et `type=timeline` (pistes et clés en JSON) ; `read_scene` décrit les deux nœuds.
* **Reste à faire** : éditeur de timeline en panneau (M5A-05), `Effect::Stop`, `timeline_sample` côté MCP, préférence « réduire les animations » (M7-07), particules et parallax (M5A-07/08).
