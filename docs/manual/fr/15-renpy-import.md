# Importer un jeu Ren'Py

OrbitEngine peut transformer un jeu Ren'Py en un nouveau projet, pour ne pas repartir de zéro en changeant de moteur. Le jeu d'origine n'est jamais modifié.

## Comment importer

- Dans l'éditeur : **Fichier > Importer depuis Ren'Py…** (aussi dans la fenêtre **Nouveau projet**). Choisissez le dossier du jeu Ren'Py (celui qui contient `game/`, ou `game/` lui-même), puis le dossier qui contiendra le nouveau projet. L'import se fait en arrière-plan ; le projet s'ouvre à la fin et le rapport d'import s'affiche.
- En ligne de commande : `orbit import-renpy <dossier_renpy> <dossier_sortie>`. Le dossier de sortie doit être vide ou absent.

Le rapport est aussi enregistré dans `import-report.md`, dans le nouveau projet : les comptes, chaque instruction non convertie (avec son fichier et sa ligne) et chaque approximation.

## Ce qui est converti

| Ren'Py | OrbitEngine |
|---|---|
| `label start:` et les autres labels de premier niveau | une scène par label, `start` en premier |
| labels imbriqués, `label .local:` | des labels dans leur scène |
| `jump`, `call`, `return` | sauts, appels et retours (un `return` hors d'un label appelé termine le jeu) |
| passage au label suivant | un saut vers cette scène |
| `menu:` avec ses choix et conditions `if` | un nœud de choix ; la question devient une ligne juste avant |
| `if` / `elif` / `else`, `while` | des conditions et des labels |
| `define e = Character("Eileen", color="#…", what_color="#…", what_prefix='"', what_suffix='"', what_italic=True, image="eileen")` | un personnage avec sa couleur ; la couleur du texte, le préfixe / suffixe, gras / italique sont écrits dans chacune de ses répliques (et dans leurs traductions) |
| `default` / `define` avec une valeur littérale (booléen, nombre, texte, liste, dictionnaire) | une variable ; `persistent.x` devient la variable persistante `persistent_x` (aussi quand elle est seulement affectée dans un label, ou seulement testée : un drapeau à `false`) |
| `$ x = …`, `$ x += …` (et `-=`, `*=`, `/=`), `$ liste.append(…)`, blocs `python:` de telles lignes | une modification de variable |
| `e "…"`, `"…"`, `"Nom" "…"` | des répliques |
| `extend "…"` | après une réplique `{nw}`, une seule réplique ; sinon une seconde réplique qui reprend la première suivie de l'ajout |
| `e happy "…"` | le sprite change d'expression, puis la réplique |
| `show eileen happy at left with dissolve`, `hide` | afficher / cacher avec position et transition |
| `at left` / `right` / `center` / `truecenter` / `top…` / `offscreenleft` / `offscreenright`, `at Transform(xalign=…, zoom=…)`, `at Position(xpos=…)` | une position de scène (prédéfinie ou libre) et une échelle ; `xzoom -1` retourne le sprite |
| `transform nom:` qui ne fait que placer le sprite (`xalign`, `yalign`, `xpos`, `ypos`, `align`, `pos`) | une position de scène nommée |
| ATL (blocs `transform`, blocs `show x:`, `at a, b`) : `linear` / `ease` / `easein` / `easeout` (et les variantes `_quad`, `_cubic`…), `pause`, `repeat [n]`, avec `xalign`, `yalign`, `xpos`, `ypos`, `xoffset`, `yoffset`, `alpha`, `zoom`, `xzoom`, `yzoom`, `rotate` | la mise en scène finale du `Show`, puis une timeline (pistes de décalage, échelle, opacité, rotation, en boucle pour `repeat`) qui ne bloque pas l'histoire |
| `scene bg room with fade`, `scene black` | décor (les sprites quittent la scène) |
| `with dissolve`, `fade`, `Dissolve(t)` (avec son `time_warp`), `Fade(a, b, c, color=…)`, `vpunch` / `hpunch`, `with Pause(t)` | fondu enchaîné, fondu (dans sa couleur), tremblement de caméra, une attente |
| `pixellate`, `ImageDissolve`, `MoveTransition`, `move…`, `wipe…`, `slide…`, `push…`, `iris…`, `zoom…` | un fondu enchaîné (signalé une fois par transition) |
| `play music … fadein 1.0`, `stop music fadeout 2.0`, `play sound`, `play voice`, `voice`, `$ renpy.music.play(…)`, `renpy.music.stop(…)`, `renpy.sound.play(…)`, `renpy.play(…)` | musique, sons, voix de la réplique suivante |
| `pause 1.0`, `$ renpy.pause(1.0)` | une attente |
| `$ renpy.jump("x")`, `renpy.call("x")`, `renpy.call_screen("x")` | un saut, un appel, l'écran ci-dessous |
| `achievement.register("x")`, `$ achievement.grant("x")` | un succès et son nœud de déblocage |
| balises `{b}`, `{i}`, `{color=…}`, `{w}`, `{p}` (`{nw}` et `{fast}` sont retirées) | le même balisage |
| `images/eileen happy.png`, `image bg room = "room.jpg"`, `Image(…)`, `im.Scale(…)` | l'expression `happy` d'Eileen, un décor |
| `image black = Solid("#000")` (ou `"#000"`) | une image générée de cette couleur (avec sa transparence) |
| `image fire:` fait seulement d'images, de `pause` et de `repeat` | une animation image par image de l'expression |
| `Composite(…)`, `ConditionSwitch(…)`, `layeredimage` | au mieux : le premier calque, la branche par défaut, le calque de base (signalé) |
| un screen avec un `imagemap` (`ground`, `hover`, `hotspot (x, y, w, h) action …`) ou des `imagebutton`, utilisé par `call screen` / `show screen` | un nœud point & click (`Hotspots`) : le `ground` devient le décor, les zones les hotspots, `hover` leur image de survol ; un `if` autour d'un hotspot devient sa condition, `sensitive` son état actif. Un `imagebutton` devient une zone image : son image `idle` est dessinée et seuls ses pixels opaques réagissent, son image `hover` la remplace au survol. Les actions faites de `SetVariable` / `ToggleVariable` (vers une valeur ou une variable), `Notify`, `Jump` et `Return` deviennent des actions de zone ; un `Call` ou une valeur calculée garde un petit bloc de nœuds générés |
| un screen de `textbutton` utilisé de la même façon | un choix |
| actions `Jump("x")`, `Call("x")`, `Return()`, `SetVariable("x", v)`, `SetField(persistent, "x", v)`, `ToggleVariable("x")`, `Notify("texte")`, et leurs listes | le bloc qui suit chaque zone / option |
| `$ renpy.notify("…")` (aussi dans un bloc `python:`) et l'action d'écran `Notify("…")` | un nœud Notify (une notification : texte converti comme un dialogue, style par défaut) |
| `tl/french/*.rpy` (blocs `strings` et blocs de dialogue) | la traduction française du projet |

Positions : le moteur ancre un sprite par le milieu de son bord bas, donc `xalign a` devient `x = 0.25 + 0.5 a` (les `left`, `center` et `right` des deux moteurs coïncident alors) et `yalign b` devient `y = 0.5 + 0.5 b` ; `xpos` / `ypos` placent l'ancre directement (les pixels sont divisés par la taille d'écran lue dans `gui.init(…)` ou `config.screen_width`). Les zones des `imagebutton` prennent la taille de leur image (en-tête PNG / JPEG).

## Ce qui n'est pas converti

- Les écrans d'interface (say, menus, sauvegarde, préférences…), les styles, les screens qui ne sont pas de la navigation, le dossier `gui/` : les écrans OrbitEngine se créent dans l'éditeur d'UI. Ils sont signalés une fois, pas ligne par ligne.
- Les transforms à paramètres, les ATL `parallel`, `choice`, `on`, `function`, `contains`, `crop` et les autres propriétés : signalés avec leur emplacement.
- Les calques des images composées et des layered images ne sont pas assemblés.
- Les blocs `init python` et le Python qui n'est ni une simple affectation ni l'un des appels ci-dessus : gardés dans un nœud de script commenté et listés dans le rapport, pour les réécrire (en Rhai, voir le chapitre Scripts).
- Les conditions que la syntaxe du moteur ne sait pas exprimer deviennent des conditions Rhai : vérifiez-les.
- L'interpolation `[variable]` reste telle quelle dans les textes ; les autres balises (`{size}`, `{font}`, `{a}`…) sont retirées (signalées une fois par balise, avec le nombre de lignes).
- Le mode NVL (les lignes vont dans la boîte de texte normale, `nvl clear` est ignoré), les side images, `window show / hide / auto`, `pause` sans durée (une seconde), les sons en boucle et les canaux autres que la musique.
- Les images WebP / AVIF et l'audio MP3 / Opus : convertissez-les en PNG / OGG, puis importez-les dans la Bibliothèque.

Après un import, ouvrez le panneau **Problèmes** : le projet est valide, mais les avertissements signalent les endroits à relire.
