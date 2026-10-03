# Variables sans code

Une **variable** retient quelque chose de l'histoire : l'argent du joueur, s'il a rencontré un personnage, quelle route il a choisie. Vous n'avez jamais besoin d'écrire du code pour en utiliser une.

## Déclarer des variables

Ouvrez le panneau **Variables** et ajoutez une variable avec un nom, un type (nombre, texte, oui/non) et une valeur de départ. Choisissez sa **portée** :

- **Partie** : fait partie de la sauvegarde du joueur ; repart de zéro avec une nouvelle partie.
- **Persistante** : retenue d'une partie à l'autre (par exemple « a vu la vraie fin »).

## Modifier une variable

Ajoutez une étape **Définir une variable** et choisissez une opération dans le menu : définir à une valeur, ajouter, soustraire, multiplier, diviser. L'Inspecteur décrit l'opération en toutes lettres, aucune formule n'est nécessaire.

## Réagir à une variable

- Ajoutez une étape **Condition** (si) : choisissez la variable, la comparaison (égal, supérieur à...) et où aller quand c'est vrai et quand c'est faux.
- Donnez une **condition à une option de choix** pour qu'elle n'apparaisse que si elle s'applique. Veillez à ce qu'au moins une option soit toujours disponible ; le panneau Problèmes vous avertit sinon.

## Afficher une variable

Les écrans personnalisés (voir le chapitre Interface) peuvent afficher une variable dans un texte, par exemple un score ou un nom.

Les variables sont vérifiées pour vous : utiliser un nom qui n'existe pas, ou mélanger texte et nombres, apparaît dans le panneau Problèmes.
