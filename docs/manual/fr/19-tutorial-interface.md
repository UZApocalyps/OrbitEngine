# Tutoriel : personnaliser l'interface du jeu

Les menus de votre jeu, la boîte de dialogue et les notifications sont dessinés à partir d'écrans et de styles que vous modifiez sans code. Ce tutoriel donne à votre jeu son propre aspect : un thème, une boîte de dialogue redessinée, des boutons en images, un menu principal avec musique, de plus beaux emplacements de sauvegarde et votre propre style de notification.

## Objectif

Remplacer l'apparence par défaut par un ensemble cohérent, et voir chaque changement en direct dans le panneau Preview.

## Avant de commencer

- Un projet dont la bibliothèque contient quelques images : une image de bouton (idéalement une pour l'état normal, une pour le survol), une image de panneau ou de cadre, et un fichier de musique. Le modèle **Démarrage visual novel** est une bonne base, car il contient déjà un jeu complet d'écrans.
- Le chapitre *Éditeur d'interface et splash* détaille chaque notion ; ce tutoriel ne fait que les parcourir dans l'ordre.

## Étapes

### 1. Ouvrir l'éditeur d'interface

1. Choisissez **Outils > Éditeur d'interface**. Le panneau **Interface** s'ouvre, avec l'**Aperçu** en direct à côté.
2. Si le panneau affiche **L'interface personnalisée est désactivée**, cliquez sur **Activer l'interface personnalisée**. Les écrans et le thème par défaut sont copiés dans le dossier `ui/` du projet, prêts à être modifiés.
3. Repérez les cinq onglets : **Écrans**, **Thème**, **Boîte de dialogue**, **Notifications** et **Splashs**. Appuyez sur **Enregistrer** en haut quand vous voulez écrire votre travail dans `ui/` ; les boutons **Annuler (interface)** et **Rétablir (interface)** défont vos modifications dans l'éditeur.

### 2. Créer un thème et un premier style

1. Ouvrez l'onglet **Thème**. Si vous voyez *Aucun fichier de thème*, cliquez sur **Créer le thème**.
2. Tapez un nom dans le champ **Nom du nouveau style**, par exemple `principal`, puis cliquez sur **Ajouter un style**.
3. Dans l'éditeur de style, passez d'un état à l'autre avec les onglets **Normal**, **Survol**, **Appui**, **Désactivé** et **Focus** : l'aperçu en haut montre le même état.
4. Pour le fond, choisissez une couleur, un **Dégradé** ou une **Image**. Pour un cadre qui garde des coins nets à toutes les tailles, choisissez l'**image 9-slice**, puis faites glisser les quatre repères sur l'image.
5. Dans **Texte**, choisissez la police, la taille et la couleur de chaque état. Utilisez d'abord **Polices** > **Ajouter une police du projet** si vous avez importé un fichier TTF ou OTF dans la bibliothèque.
6. Faites un second style qui part du premier : ajoutez `principal_grand`, réglez **Hérite de** sur `principal` et ne changez que la taille. Les champs qu'il modifie sont mis en évidence, les autres sont marqués *hérité*.

### 3. Boutons en images

1. Créez un style `bouton_image`. Pour chaque onglet d'état (**Normal**, **Survol**, **Appui**), mettez le fond sur **Image** et choisissez l'image correspondante. Un état laissé vide reprend l'apparence du survol, ou la normale.
2. Si l'image contient déjà le libellé, désactivez **Afficher le texte** : le bouton devient purement visuel, et les lecteurs d'écran lisent toujours le texte.
3. Dans l'onglet **Écrans**, sélectionnez un écran, puis un bouton dans l'arbre des **Widgets**, et cliquez sur **Appliquer à** (le nom du widget) dans l'éditeur de style pour lui donner ce style. Vous pouvez aussi choisir le style dans le champ **Style** du widget, ou régler des images pour ce seul bouton avec les **Surcharges locales**.
4. Pour rendre une décoration cliquable, ajoutez un widget **Image** et donnez-lui au moins une action dans **Actions au clic**.

### 4. La boîte de dialogue

1. Ouvrez l'onglet **Boîte de dialogue**. Choisissez **Une réplique** ou **Choix** dans **Aperçu :** et saisissez un nom et un texte d'exemple.
2. Dans **Boîte de texte**, réglez le placement, la taille et les marges, puis donnez-lui un **style de boîte** : utilisez le **+** à côté du champ de style pour en créer un à partir des couleurs actuelles, ou choisissez votre style de cadre.
3. Dans **Nom du personnage**, choisissez la source de la couleur et, si vous le souhaitez, une **plaque du nom** avec son propre style.
4. Dans **Choix**, réglez le style des boutons : leur apparence au survol marque le choix sélectionné.
5. Vérifiez le résultat dans le panneau Preview avec une vraie réplique : sélectionnez une étape et appuyez sur **Ici** (F7).

### 5. Un menu principal avec sa musique

1. Dans l'onglet **Écrans**, cliquez sur **Nouvel écran à partir de...** et choisissez **Menu principal** pour partir du modèle (ou ouvrez l'écran de menu principal existant).
2. Sélectionnez l'écran. Dans ses propriétés, **Remplace** indique le rôle de l'écran dans le jeu : choisissez **menu principal**.
3. Ajoutez des boutons avec le sélecteur de type de widget et le bouton d'ajout au-dessus de l'arbre des **Widgets**. Pour chaque bouton, ouvrez **Actions**, cliquez sur **Ajouter une action** et choisissez ce qu'il fait : **Nouvelle partie**, **Continuer**, **Ouvrir un écran**, **Quitter**.
4. Appliquez votre style `bouton_image` aux boutons.
5. Sans rien sélectionner dans l'arbre (l'écran lui-même), cochez **Musique de l'écran**. Choisissez le **Fichier audio**, puis réglez le **Volume**, les fondus et **Jouer la piste en boucle**.
6. Cochez **Continuer si l'écran suivant a la même piste** pour que l'ouverture des préférences ne relance pas la chanson. Pour le menu pause, **Par-dessus le jeu** propose **Remplacer la musique du jeu** ou **Baisser la musique du jeu**.

### 6. Emplacements de sauvegarde

1. Sélectionnez l'écran de sauvegarde (**Remplace** : **sauvegarde**), puis son widget **Emplacements de sauvegarde**. Réglez **Emplacements** sur **Sauvegarder dans un emplacement**.
2. Dans **Apparence des emplacements**, choisissez un **Style de la carte**, un **Style de la carte vide**, le **Style des onglets**, le **Style des boutons de page** et le **Style du bouton Supprimer**. Ajustez la **Hauteur de la carte**, l'**Espace entre colonnes** et l'**Espace entre lignes**, ainsi que le nombre de colonnes et de lignes.
3. Pour un contrôle total, cliquez sur **Personnaliser la carte**. La carte par défaut est écrite sous forme de widgets : déplacez la miniature, ajoutez un texte de **temps de jeu** lié à un **Champ de l'emplacement**, changez les polices. **Revenir à la carte par défaut** annule vos changements.
4. Utilisez les conditions **l'emplacement est vide** et **l'emplacement contient une sauvegarde** pour n'afficher un texte « Vide » ou un cadenas qu'au bon endroit.

### 7. Notifications

1. Ouvrez l'onglet **Notifications**. Cliquez sur **Afficher une notification de test** pour voir une bulle sur l'aperçu.
2. Réglez l'ancrage, la largeur et l'animation. Sous la liste des styles, tapez un nom dans **nom du nouveau style** et cliquez sur **Ajouter un style de notification**.
3. Modifiez son **Fond**, sa **Bordure**, l'**Arrondi des coins**, la **Taille de l'icône**, la **Taille du texte** et la **Couleur du texte**. Les champs laissés vides reprennent le style `default`.
4. Dans l'histoire, une étape **Notification** choisit le style par son nom. Les succès utilisent le style `achievement` et les messages du moteur le style `system`.

### 8. Un écran de démarrage

1. Ouvrez l'onglet **Splashs**. Le splash *OrbitEngine (obligatoire)* passe toujours en premier.
2. Cliquez sur **Ajouter un splash**, puis **Image du projet** ou **Splash de texte**. Réglez la **Durée**, le **Fondu d’entrée** et le **Fondu de sortie**, et décidez s'il **Peut être passé**.
3. Cliquez sur **Aperçu des splashs** pour les voir.

## Résultat

Le jeu démarre avec votre splash, puis un menu principal avec vos boutons et votre musique. La boîte de dialogue, les emplacements de sauvegarde et les notifications partagent une même identité visuelle, définie par une poignée de styles.

## Pour aller plus loin

- Ajoutez une galerie d'images débloquées, un menu de replay ou un écran de succès (voir *Éditeur d'interface et splash*).
- Donnez à chaque langue sa police dans **Thème > Polices par langue** (voir *Traductions*).
- Construisez un inventaire ou un téléphone sous forme d'écran ouvert par l'histoire avec **Ouvrir un écran**.
- Passez le dialogue en bulles de BD : tutoriel *Tutoriel : des personnages expressifs*.

## En cas de problème

- **Le jeu affiche toujours les menus par défaut** : le dossier `ui/` n'est utilisé que s'il existe. Activez l'interface personnalisée, puis appuyez sur **Enregistrer**.
- **Un bouton ne fait rien** : vérifiez ses **Actions**. Un bouton qui pointe vers un écran inexistant est signalé par le panneau Interface (**Validation**).
- **Le texte d'un bouton-image est en double** : désactivez **Afficher le texte** dans le style.
- **Ma musique ne se lance pas** : le champ **Fichier audio** affiche un avertissement quand le fichier n'est pas dans le projet. Importez-le d'abord dans la bibliothèque.
- **Une modification a disparu après un rechargement** : les modifications d'interface non enregistrées sont abandonnées quand vous rechargez le dossier `ui/` ou changez de projet. Appuyez souvent sur **Enregistrer**.
