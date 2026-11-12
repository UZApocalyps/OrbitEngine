# Variables sans code

Une **variable** retient quelque chose de l'histoire : l'argent du joueur, s'il a rencontré un personnage, quelle route il a choisie. Vous n'avez jamais besoin d'écrire du code pour en utiliser une.

## Déclarer des variables

Ouvrez le panneau **Variables** et ajoutez une variable avec un nom, un type (nombre, texte, oui/non) et une valeur de départ. Choisissez sa **portée** :

- **Partie** : fait partie de la sauvegarde du joueur ; repart de zéro avec une nouvelle partie.
- **Persistante** : retenue d'une partie à l'autre (par exemple « a vu la vraie fin »).

## Modifier une variable

Ajoutez une étape **Modifier une variable** et choisissez une opération dans le menu : définir à une valeur, ajouter, soustraire, multiplier, diviser, inverser (oui/non). L'Inspecteur décrit l'opération en toutes lettres, aucune formule n'est nécessaire. Le **Mode expression** permet de saisir une formule à la place (une expression qui peut utiliser d'autres variables) et **Revenir au mode visuel** ramène aux menus. Une zone cliquable peut aussi modifier des variables au clic, avec les mêmes opérations (voir *Écrire l'histoire*).

## Réagir à une variable

- Ajoutez une étape **Condition** (si) : choisissez la variable, la comparaison (égal, supérieur à...) et où aller quand c'est vrai et quand c'est faux.
- Donnez une **condition à une option de choix** pour qu'elle n'apparaisse que si elle s'applique. Veillez à ce qu'au moins une option soit toujours disponible ; le panneau Problèmes vous avertit sinon.

## Afficher une variable

Les écrans personnalisés (voir le chapitre Interface) peuvent afficher une variable dans un texte, par exemple un score ou un nom.

Les variables sont vérifiées pour vous : utiliser un nom qui n'existe pas, ou mélanger texte et nombres, apparaît dans le panneau Problèmes.

## Listes et tables

Une variable peut aussi contenir une **liste** (une collection ordonnée : les objets d'un inventaire) ou une **table** (des entrées avec une clé : l'affinité de chaque personnage). Leur contenu se modifie dans le panneau Variables (**+ élément**, **+ entrée**). Opérations : ajouter à la liste, retirer une valeur, retirer à une position, vider, définir ou retirer l'entrée d'une clé. Dans une condition, on peut tester si elle **contient** une valeur, **a la clé**, ou comparer sa **taille**. Un écran peut en afficher le contenu avec une liste générée (voir le chapitre *Interface*).

## Construire des conditions

Une condition (étape Condition, option de choix, zone cliquable, widget) se construit avec des lignes : une variable, une comparaison (égal, supérieur à, est vrai, contient...) et une valeur. Les lignes sont regroupées par **Toutes ces conditions sont vraies** ou **Au moins une est vraie**, et **Ajouter un groupe** imbrique des groupes. Pour une condition que les lignes ne savent pas exprimer, passez à une expression typée (`affinite > 2 && !vu`) ou cochez **Rhai** (voir *Scripts*).
