# Éditeur d'interface et splash

Les menus de votre jeu (menu principal, pause, sauvegarde, chargement, préférences) sont des **écrans** que vous pouvez restyler sans code. Ils vivent dans le dossier `ui/` de votre projet. Sans ce dossier, le jeu utilise les écrans par défaut.

## L'éditeur d'interface

Ouvrez le panneau **Interface**. Il comporte trois onglets :

- **Écrans** : choisissez un écran et modifiez-le. Ajoutez des boutons, des textes, des images et des conteneurs, déplacez-les, et réglez l'action d'un bouton (lancer le jeu, ouvrir un autre écran, quitter...). Un aperçu en direct de la vraie interface du jeu est affiché à côté de l'éditeur.
- **Thème** : couleurs, polices et apparence commune à tous les écrans.
- **Splashes** : les écrans affichés au démarrage du jeu.

Le modèle **Démarrage visual novel** contient déjà un jeu complet d'écrans à adapter. Les modifications sont enregistrées dans le dossier `ui/` ; annuler et rétablir fonctionnent comme d'habitude. Le panneau Interface liste les problèmes (par exemple un bouton qui pointe vers un écran inexistant).

## Les splashes

Au démarrage, le jeu affiche d'abord le splash OrbitEngine. Celui-ci est obligatoire et ne peut être ni retiré ni modifié. Ensuite, vous pouvez ajouter vos propres splashes : un texte ou une image de votre projet, avec une durée, des temps d'apparition et de disparition en fondu, une couleur de fond, et la possibilité ou non de le passer. Utilisez **Aperçu des splashes** pour les visionner.

## Écrans ouverts par l'histoire, HUD, listes générées et transitions

Les menus ne sont pas les seuls écrans. L'histoire elle-même peut ouvrir un écran de votre projet à tout moment : un inventaire, un téléphone, une carte, un compteur de points gardé dans un coin. Ces écrans se créent dans le même éditeur d'interface et disposent de quelques réglages supplémentaires décrits ici.

### L'étape « Ouvrir un écran »

Ajoutez une étape **Ouvrir un écran** dans une scène et choisissez un écran du projet par son identifiant. Trois réglages décident de son comportement :

- **Attendre** : coché, l'histoire s'arrête sur cette étape jusqu'à ce que le joueur ferme l'écran. Elle se comporte alors exactement comme un choix : un retour en arrière rouvre l'écran, une sauvegarde retient qu'il est ouvert, et le passage rapide s'arrête là puisque c'est une décision du joueur. Décoché, l'écran s'ouvre et l'histoire continue aussitôt (le réglage habituel d'un HUD).
- **Résultat** : une variable de l'histoire qui reçoit la valeur avec laquelle l'écran se ferme (voir plus bas). Laissez-la vide si l'écran ne renvoie rien.
- **Paramètres** : une liste de noms, chacun avec une valeur littérale ou une variable. L'écran les affiche grâce à la liaison **Paramètre d'écran** ; un seul écran « téléphone » peut ainsi montrer des contacts différents selon l'endroit où il est ouvert.

Ouvrir un écran déjà ouvert le remplace. Si l'identifiant n'existe pas dans le projet, le jeu ferme l'écran automatiquement et le panneau **Problèmes** le signale : l'histoire ne reste jamais bloquée.

### L'étape « Fermer un écran »

**Fermer un écran** ferme un écran ouvert par l'histoire, typiquement un HUD devenu inutile. Rien ne se passe si l'écran n'est pas ouvert.

### Fermer avec une valeur

L'action **Fermer** d'un bouton possède une **valeur de retour** facultative : une valeur littérale, une variable, l'élément de la ligne, l'index de la ligne, la clé de la ligne (voir les listes générées) ou un paramètre d'écran. Quand l'écran a été ouvert avec une variable **Résultat**, cette valeur y est rangée. Fermer avec Échap ou par les actions d'annulation de l'écran ne renvoie aucune valeur : la variable garde son contenu précédent. Un écran ouvert par l'histoire sans actions d'annulation laisse Échap ouvrir le menu de pause.

### Les écrans HUD

Cochez la propriété **HUD** d'un écran pour en faire un affichage tête haute. Un HUD est dessiné sous la boîte de texte, ne met jamais le jeu en pause et ne prend jamais le focus clavier. Les clics en dehors de ses boutons vont au jeu. Il est masqué quand l'interface est cachée et dès qu'un menu (pause, sauvegarde...) est ouvert. Plusieurs HUD peuvent être ouverts en même temps.

### Les listes générées

Le widget **Répéter** construit une liste à partir d'une variable liste ou map de l'histoire, ou d'un paramètre d'écran. Ses enfants sont le **gabarit d'une ligne** : ils sont recopiés une fois par élément. Dans le gabarit, utilisez les liaisons **Élément** (l'élément lui-même), **Index de l'élément** (sa position, à partir de 0) et **Clé de l'élément** (pour les maps). Les actions d'une ligne peuvent aussi utiliser l'élément, par exemple « utiliser cet objet » : un bouton qui ferme l'écran avec l'objet, ou qui le retire de la variable d'inventaire.

Le texte **vide** s'affiche quand la collection n'a aucun élément. Choisissez la **direction** (verticale, horizontale ou une grille avec un nombre de colonnes). Une liste affiche au plus 500 lignes ; une map présente ses entrées par ordre de clé.

### Les transitions d'écran

Chaque écran peut avoir une **transition d'ouverture** et une **transition de fermeture** : fondu, glissement vers le haut, le bas, la gauche ou la droite, ou mise à l'échelle, avec une durée et une courbe d'accélération (les courbes nommées de votre projet sont acceptées). Les transitions sont purement visuelles : l'écran est ouvert ou fermé immédiatement, seul son dessin est animé. Le retour en arrière, le chargement d'une sauvegarde et la préférence d'accessibilité **réduire les animations** les coupent.

### L'exemple de la démo

L'histoire de démonstration (`examples/stories`) utilise les deux sortes d'écran :

- Quand Alice quitte la boutique, une étape **Ouvrir un écran** avec **Attendre** ouvre l'écran `inventory` avec le paramètre `owner` réglé sur « Alice ». L'écran répète la liste `inventory` ; chaque bouton ferme l'écran avec son objet, qui atterrit dans la variable `used_item`. Le bouton **Retour** et Échap ferment sans valeur. Il glisse vers le haut à l'ouverture et disparaît en fondu à la fermeture.
- Au début du jeu, une étape **Ouvrir un écran** sans **Attendre** ouvre le HUD `status` : un petit panneau en haut à droite qui affiche la variable `credits`, mise à jour au fil de l'histoire.
