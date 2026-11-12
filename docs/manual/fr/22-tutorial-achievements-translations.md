# Tutoriel : succès, notifications et traductions

Trois finitions qui donnent à un jeu un air d'abouti : des récompenses que le joueur collectionne, de courts messages qui apparaissent sans arrêter l'histoire, et une seconde langue. Elles sont indépendantes : ne suivez que les parties dont vous avez besoin.

## Partie 1. Les succès

### Objectif

Définir deux succès, l'un visible et l'autre caché, les débloquer depuis l'histoire, et voir la notification et l'écran des succès du jeu.

### Avant de commencer

- Un projet dont l'histoire a au moins une scène et un choix (celle du *Tutoriel : votre premier visual novel* convient parfaitement).
- Deux petites icônes PNG carrées, si vous voulez les vôtres (une médaille par défaut est utilisée sinon).

### Étapes

1. Ouvrez **Affichage > Panneaux > Édition > Afficher : Succès**. Le panneau **Succès** s'ouvre.
2. Cliquez sur **Ajouter**. Un **Nouveau succès** apparaît dans la liste. Remplissez le **Titre** (par exemple `Premiers mots`), la **Description** (ce que le joueur a fait) et, si vous en avez une, une **Icône**.
3. Ajoutez-en un second, `Généreux`, pour le joueur qui a partagé son déjeuner. Cochez **Caché tant qu'il n'est pas débloqué** : la liste affichera un texte neutre à la place de son titre, de sa description et de son icône jusqu'à son déblocage. Vous pouvez aussi définir une **Icône verrouillée**, affichée tant que le succès n'est pas débloqué.
4. Débloquez-les maintenant. Dans le panneau **Scène**, sélectionnez l'étape après laquelle le joueur gagne le premier (par exemple la première réplique de l'histoire), puis insérez une étape **Débloquer un succès** : appuyez sur Ctrl+I et tapez `débloquer`, ou utilisez le bouton **Ajouter un nœud de déblocage ici** du panneau Succès, qui l'insère pour vous.
5. Dans l'**Inspecteur**, choisissez le **Succès** à débloquer. Faites de même sous l'étiquette de la branche généreuse pour `Généreux`.
6. Le panneau affiche, sous chaque succès, **Débloqué par** les étapes qui le débloquent. Le message **Aucun nœud ne débloque encore ce succès.** vous avertit d'un oubli ; le panneau **Problèmes** le signale aussi.
7. Jouez avec **Preview** (F6). Quand l'histoire atteint l'étape, une notification annonce le succès. L'aperçu n'enregistre jamais les déblocages : vous pouvez rejouer autant que vous voulez.

### Résultat

Des succès qui se débloquent une seule fois, sont retenus d'une partie à l'autre (même si le joueur efface ses sauvegardes), et sont listés sur l'écran **Succès** ouvert depuis le menu principal (il n'apparaît que si l'histoire définit des succès).

### Pour aller plus loin

- Les titres et descriptions des succès sont des textes ordinaires : ils se traduisent avec le reste (voir la partie 3).
- Redessinez la notification avec le style `achievement` (partie 2) et l'écran des succès dans le panneau **Interface**.
- Construisez votre propre écran avec une liste générée dont la **Liste** est **Succès** (voir *Éditeur d'interface et splash*).

### En cas de problème

- **Rien ne se passe à l'étape** : l'étape ne compte que la première fois, dans une vraie partie. Dans un replay de scène, elle ne fait rien.
- **L'étape indique « Succès inconnu »** : le succès visé a été supprimé. Choisissez-en un autre.
- **L'icône est refusée** : elle doit être une image déclarée dans le projet ; importez-la d'abord dans la bibliothèque.

## Partie 2. Notifications

### Objectif

Afficher un court message dans un coin de l'écran, dans un style bien à vous, sans jamais arrêter l'histoire.

### Étapes

1. Dans le panneau Scène, sélectionnez une étape et insérez une étape **Notification** (appuyez sur Ctrl+I et tapez `notif`).
2. Dans l'**Inspecteur**, écrivez le texte. Vous pouvez utiliser des balises comme `{b}important{/b}` et mettre une variable entre crochets, par exemple `Amitié : [amitie]` : sa valeur s'affiche quand la notification apparaît.
3. Choisissez si vous voulez une **Icône** (une image), un **Son**, une **Durée propre** en secondes et un **Style**. La liste des styles vient du thème ; **ou un style du thème…** accepte le nom d'un style que vous avez créé.
4. Pour dessiner l'apparence, ouvrez **Outils > Éditeur d'interface**, onglet **Notifications**. Réglez l'ancrage, la largeur, l'animation et le nombre maximal de notifications. Dans la liste des styles, tapez un nom dans **nom du nouveau style** et cliquez sur **Ajouter un style de notification** (par exemple `indice`). Stylez son **Fond**, sa **Bordure**, l'**Arrondi des coins**, la **Taille du texte** et la **Couleur du texte**, puis cliquez sur **Afficher une notification de test** pour la voir sur l'aperçu.
5. De retour dans l'étape Notification, choisissez votre style. Les succès utilisent le style `achievement` et les messages du moteur le style `system` : les redessiner les change partout.

### Résultat

De courts messages qui ne bloquent jamais l'histoire. Ils ne sont jamais sauvegardés : revenir en arrière ou charger une sauvegarde retire les notifications affichées à l'écran.

### Pour aller plus loin

- Une zone d'une carte point & click ou un bouton d'un écran peut aussi en afficher une, avec l'action **Notification** (voir *Tutoriel : point & click, explorer une maison*).
- Dans **Pendant l'avance rapide**, décidez si les notifications sont affichées, abrégées ou masquées pendant que le joueur passe le texte.
- Un texte écrit dans une notification se traduit comme tous les autres.

## Partie 3. Traduire votre jeu

### Objectif

Ajouter une seconde langue et l'amener à 100 %, avec l'aide d'un tableur.

### Étapes

1. Ouvrez **Affichage > Panneaux > Édition > Afficher : Traductions**. Le panneau **Traductions** s'ouvre.
2. Dans le champ **nouvelle langue**, tapez un code de langue (`en`, `es`, `pt-BR`...) et cliquez sur **Créer**. Le fichier `i18n/story.<code>.ron` est créé à côté de votre histoire. La langue dans laquelle vous écrivez est la langue source et ne peut pas être créée comme traduction.
3. Le panneau affiche la couverture, par exemple *0/42 traduites, 42 manquantes, 0 orphelines*. Tous les textes comptent : répliques, choix, noms de personnages, textes de notifications, titres et descriptions de succès.
4. Pour traduire dans un tableur, cliquez sur **Exporter en CSV...** (ou **Exporter en PO...** pour les outils de traduction comme Poedit, ou passez par **Fichier > Import, export et traductions**). Le panneau confirme combien de chaînes ont été exportées et où.
5. Envoyez le fichier à votre traducteur, ou remplissez-le vous-même. Gardez le format du fichier et les identifiants.
6. Quand il revient, cliquez sur **Importer un PO/CSV...**. Le panneau indique combien de chaînes ont été mises à jour et combien sont périmées. Le type de fichier est reconnu d'après son extension (`.po` ou `.csv`).
7. Vérifiez la couverture : les *manquantes* doivent tomber à 0. Si vous réécrivez plus tard un texte source, sa traduction est signalée dans **Traductions périmées** ; relisez chacune, ou cliquez sur **Tout marquer comme relu** si elles sont toujours correctes. **Considérer à jour** sert aux projets traduits avant l'existence de cette vérification.
8. Pour une langue avec son propre alphabet (chinois, japonais, coréen, cyrillique...), importez une police qui contient les caractères (TTF ou OTF dans la bibliothèque), puis ouvrez **Outils > Éditeur d'interface**, onglet **Thème** : ajoutez la police dans **Polices** et ajoutez-la à la langue dans **Polices par langue**.
9. Testez dans un jeu exporté : dans le panneau Export, réglez **Langue par défaut** sur votre nouveau code, exportez, et jouez. Le joueur peut aussi changer de langue dans l'écran **Préférences** du jeu.

### Résultat

Un jeu qui démarre dans la langue de votre choix et propose les autres dans ses préférences, avec une traduction que vous pouvez mettre à jour chaque fois que l'histoire change.

### Pour aller plus loin

- Dans le panneau **Interface**, le sélecteur de langue en haut affiche les textes des menus (`ui.*`) en anglais, en français ou dans la langue de l'histoire, pour voir l'aspect de vos écrans dans une autre langue.
- Ajoutez des voix par langue avec **Importer les voix…** sur une réplique (voir *Personnages et animations* et l'Inspecteur).
- Gardez le CSV sous gestion de versions : son diff montre exactement quels textes ont changé.

### En cas de problème

- **« Type de fichier inconnu »** : seuls les fichiers `.po` et `.csv` peuvent être importés.
- **Des caractères s'affichent en carrés** : aucune police de la langue ne les contient. L'éditeur d'interface et le journal d'export indiquent quels caractères manquent.
- **Certains textes restent dans la langue source** : ils sont comptés comme *manquantes* dans le panneau ; remplissez-les et importez de nouveau.
- **Le panneau ne me laisse pas saisir les traductions** : l'édition se fait hors de l'éditeur (tableur ou outil PO) puis s'importe.
