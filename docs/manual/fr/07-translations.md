# Traductions (PO et CSV)

Chaque texte de votre histoire (répliques, noms de personnages, choix) peut être traduit dans d'autres langues. La langue dans laquelle vous écrivez est la **langue source**.

## Ajouter une langue

Ouvrez le panneau **Traductions**, saisissez un code de langue (par exemple `fr` ou `pt-BR`) et appuyez sur **Créer**. Le panneau indique l'avancement de la traduction : combien de textes sont traduits, combien manquent, et combien n'existent plus dans l'histoire (orphelins).

## Traduire

Deux façons de travailler :

- **Dans l'éditeur** : saisissez le texte traduit à côté de chaque texte source.
- **Dans un autre outil** : exportez la traduction en fichier **PO** (utilisé par de nombreux outils de traduction) ou **CSV** (s'ouvre dans n'importe quel tableur), envoyez-le à votre traducteur, puis **Importez** le résultat. Le type de fichier est reconnu par son extension (.po ou .csv). L'import met à jour les textes et indique combien ont changé.

## Traductions périmées

Si vous réécrivez un texte source après sa traduction, la traduction est marquée **périmée** : elle ne correspond peut-être plus. Le panneau les liste pour que vous les revoyiez une à une, ou appuyez sur **Tout marquer comme relu** quand elles sont toujours justes. Pour les projets traduits avant cette fonction, **Adopter comme à jour** enregistre les textes sources actuels comme référence.

## Polices pour les autres langues

Les textes sont dessinés avec des polices qui gèrent la langue, y compris les caractères chinois, japonais et coréens, avec un repli quand un glyphe manque.

## Polices par langue

La police par défaut ne couvre que les écritures latines. Pour une autre langue, importez une police qui contient ses caractères (Bibliothèque, TTF ou OTF), puis ouvrez l'éditeur d'interface, onglet **Thème** : ajoutez la police dans **Polices**, et dans **Polices par langue** ajoutez-la à la langue. La liste est une chaîne de repli, police préférée en premier. Elle s'applique à la boîte de dialogue, aux choix et à tous les menus. Un avertissement apparaît là, et dans le journal d'export, quand des caractères d'une traduction ne peuvent être dessinés par aucune police de sa langue.

Les lignes japonaises et chinoises passent à la ligne entre les caractères sans besoin d'espaces, et une ponctuation comme `。` ou `」` n'est jamais seule en début de ligne. Les textes arabes et hébreux sont mis en forme et ordonnés de droite à gauche, et alignés à droite.

## Dans le jeu

Le joueur choisit la langue dans l'écran des préférences ; sans choix, le jeu démarre dans la langue définie dans le profil d'export.
