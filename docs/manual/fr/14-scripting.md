# Scripts (Rhai)

Les scripts sont facultatifs : on peut faire un jeu entier sans eux. Quand le graphe devient trop lourd pour une logique (une boutique, un jeu de dés, un score avec des règles), vous pouvez ajouter un peu de code en **Rhai**, un langage simple proche de JavaScript ou de Rust. Les scripts s'exécutent dans un **bac à sable** : ils n'ont accès ni aux fichiers, ni au réseau, ni à l'horloge, et ils s'arrêtent d'eux-mêmes s'ils tournent trop longtemps.

## L'étape Script

Insérez une étape **Script** (groupe Logique du menu d'insertion). Son inspecteur affiche un éditeur de code avec coloration. Les erreurs s'affichent dessous et dans le panneau **Problèmes**, avec leur ligne et leur colonne ; un clic y mène.

Ce qu'un script peut faire :

| Fonction | Effet |
|---|---|
| `get_var("or")` | lit une variable du projet |
| `has_var("or")` | indique si la variable existe |
| `set_var("or", 12)` | modifie une variable (appliqué à la fin du script) |
| `rand_int(1, 6)`, `rand_float()`, `chance(0.3)` | nombres aléatoires tirés du générateur du jeu |
| `branch("gagne")` | choisit la sortie `gagne` (renvoyer le texte `"gagne"` fait de même) |
| `log("texte")` | écrit un message dans la console (aussi `print`) |

Les variables sont celles déclarées dans le panneau **Variables** ; un script ne peut pas en créer. Leur type est conservé : une variable entière reste entière.

### Sorties

Une étape Script a des **branches** nommées, chacune vers une étiquette, et une étiquette d'**erreur** facultative. Si le script choisit une branche, l'histoire y saute ; sinon elle continue avec l'étape suivante. Si le script échoue (erreur, mauvais type, limite atteinte), **rien de ce qu'il a fait n'est appliqué** et l'histoire va à l'étiquette d'erreur, ou simplement à l'étape suivante, avec un avertissement.

### Tester

Le bouton **Exécuter dans le bac à sable** lance le script avec les valeurs initiales des variables et montre la branche choisie, les variables modifiées (avant et après), ses messages et le nombre de tirages aléatoires. Le projet n'est pas modifié.

## Conditions Rhai

La condition d'une étape **Condition** ou d'une option de choix peut s'écrire en Rhai : cochez **Rhai** à côté de la condition. Une condition Rhai ne fait que lire (`get_var("or") % 2 == 0 && get_var("manche") < 3`) ; elle ne peut ni modifier une variable ni tirer au hasard. Si elle échoue, elle vaut faux. Dans la vue texte, elle s'écrit `rhai "…"`.

## Modules partagés

Le code utilisé par plusieurs scripts va dans un **module** : un fichier `scripts/<nom>.rhai` du projet, modifiable dans le panneau **Scripts**. Un script l'utilise ainsi :

```rhai
import "des" as des;
des::parier("plus")
```

L'exemple `examples/stories/test_script.ron` (un duel de dés en trois manches) et son module `examples/stories/scripts/dice.rhai` montrent un module, un script qui choisit sa branche et des conditions Rhai.

## Sauvegardes, retour arrière et déterminisme

Un script ne garde rien pour lui : tout ce qui compte est dans des variables, et ses nombres aléatoires viennent du générateur du jeu, qui est sauvegardé. Charger une sauvegarde ou revenir en arrière puis rejouer donne exactement le même résultat, sur ordinateur comme sur le web.

## Limites

Pour protéger les joueurs, un script s'arrête en erreur au-delà de 100 000 opérations, de 32 niveaux d'appels de fonctions, d'un texte de plus de 64 Ko ou d'une liste ou table de plus de 10 000 éléments.

## Export

L'export embarque les modules importés par vos scripts. L'export web utilise un lecteur un peu plus gros quand le jeu a des scripts (environ 0,9 Mo de plus, 230 Ko une fois compressé) ; les jeux sans script gardent le petit. Les lecteurs ordinateur et Android gèrent toujours les scripts.

## Pour l'assistant (MCP)

Les outils `list_scripts`, `read_script`, `set_script`, `write_script_module`, `delete_script_module`, `check_scripts` et `test_script` lisent, modifient, vérifient et testent les scripts ; `add_command` avec `type: script` ajoute une étape Script.
