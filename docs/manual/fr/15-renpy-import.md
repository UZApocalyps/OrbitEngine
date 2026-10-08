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
| `define e = Character("Eileen", color="#…")` | un personnage avec sa couleur |
| `default` / `define` avec une valeur littérale (booléen, nombre, texte, liste, dictionnaire) | une variable ; `persistent.x` devient la variable persistante `persistent_x` |
| `$ x = …`, `$ x += …` (et `-=`, `*=`, `/=`), `$ liste.append(…)` | une modification de variable |
| `e "…"`, `"…"`, `"Nom" "…"`, `extend "…"` | des répliques |
| `e happy "…"` | le sprite change d'expression, puis la réplique |
| `show eileen happy at left with dissolve`, `hide` | afficher / cacher avec position et transition |
| `scene bg room with fade`, `scene black` | décor (les sprites quittent la scène) |
| `with dissolve`, `fade`, `Dissolve(t)`, `Fade(a, b, c)`, `pixellate`, `vpunch` / `hpunch` | fondu enchaîné, fondu au noir, tremblement de caméra |
| `play music … fadein 1.0`, `stop music fadeout 2.0`, `play sound`, `play voice`, `voice` | musique, sons, voix de la réplique suivante |
| `pause 1.0` | une attente |
| balises `{b}`, `{i}`, `{color=…}`, `{w}`, `{p}` | le même balisage |
| `images/eileen happy.png`, `image bg room = "room.jpg"` | l'expression `happy` d'Eileen, un décor |
| `tl/french/*.rpy` (blocs `strings` et blocs de dialogue) | la traduction française du projet |

## Ce qui n'est pas converti

- Screens, styles, transforms et ATL, layered images, le dossier `gui/` : les écrans OrbitEngine se créent dans l'éditeur d'UI.
- Les blocs `init python` et le Python qui n'est pas une simple affectation : gardés dans un nœud de script commenté et listés dans le rapport, pour les réécrire (en Rhai, voir le chapitre Scripts).
- Les conditions que la syntaxe du moteur ne sait pas exprimer deviennent des conditions Rhai : vérifiez-les.
- L'interpolation `[variable]` reste telle quelle dans les textes ; les autres balises (`{size}`, `{font}`, `{a}`…) sont retirées.
- Le mode NVL (les lignes vont dans la boîte de texte normale), `pause` sans durée (une seconde), les sons en boucle et les canaux autres que la musique.
- Les images WebP / AVIF et l'audio MP3 / Opus : convertissez-les en PNG / OGG, puis importez-les dans la Bibliothèque.

Après un import, ouvrez le panneau **Problèmes** : le projet est valide, mais les avertissements signalent les endroits à relire.
