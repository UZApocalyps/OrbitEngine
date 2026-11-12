# Éditeur d'interface et splash

Les menus de votre jeu (menu principal, pause, sauvegarde, chargement, préférences) sont des **écrans** que vous pouvez restyler sans code. Ils vivent dans le dossier `ui/` de votre projet. Sans ce dossier, le jeu utilise les écrans par défaut.

## L'éditeur d'interface

Ouvrez le panneau **Interface**. Il comporte cinq onglets :

- **Écrans** : choisissez un écran et modifiez-le. Ajoutez des boutons, des textes, des images et des conteneurs, déplacez-les, et réglez l'action d'un bouton (lancer le jeu, ouvrir un autre écran, quitter...). Un aperçu en direct de la vraie interface du jeu est affiché à côté de l'éditeur.
- **Thème** : les polices et les **styles** nommés communs à tous les écrans, dans un éditeur de styles visuel.
- **Boîte de dialogue** : la boîte de texte, le nom du personnage, les choix et le menu rapide affichés pendant le jeu, et le choix entre la boîte classique et les bulles de bande dessinée.
- **Notifications** : où et comment apparaissent les notifications du jeu.
- **Splashes** : les écrans affichés au démarrage du jeu.

Le modèle **Démarrage visual novel** contient déjà un jeu complet d'écrans à adapter. Les modifications sont enregistrées dans le dossier `ui/` ; annuler et rétablir fonctionnent comme d'habitude. Le panneau Interface liste les problèmes (par exemple un bouton qui pointe vers un écran inexistant).

## Les styles

Un **style** est une apparence nommée (fond, bordure, ombre, marge intérieure, police, couleur du texte, sons) que tout widget peut utiliser. Sélectionnez un style dans l'onglet **Thème** pour le modifier :

- L'**aperçu** en haut montre un bouton, une étiquette, un panneau, un curseur, une case et une liste déroulante dessinés avec le style. Les **onglets d'état** (Normal, Survol, Appuyé, Désactivé, Focus) changent à la fois l'aperçu et les champs modifiés : chaque état peut avoir son propre fond et sa couleur de texte. Un état laissé vide reprend l'aspect survolé (appuyé et focus) ou normal.
- **Fond** : aucun, une couleur, un **dégradé** vertical, une **image** (étirée, ajustée ou couvrante) ou une image **9-slice**. Pour un 9-slice, faites glisser les quatre repères sur l'image pour choisir les bordures qui gardent leur taille (les coins restent nets, le centre s'étire). Les couleurs ont un canal alpha.
- **Boîte** : couleur et épaisseur de la bordure, **ombre** portée (couleur et décalage), marge intérieure. Les marges extérieures se règlent sur chaque widget (section Disposition).
- **Texte** : police, taille, gras, alignement et couleur de l'état courant. Désactivez **Afficher le texte** pour un bouton image seule (les lecteurs d'écran lisent toujours le texte).
- **Sons** joués au survol et au clic.
- **Étend** : un style peut partir d'un autre. Les champs que le style change sont en surbrillance ; les autres affichent la valeur du parent, marquée *hérité*. Le bouton de réinitialisation à côté d'un champ le rend au parent.
- **Dupliquez**, renommez ou supprimez un style depuis son en-tête. **Utilisé par** liste les widgets, styles et éléments de la boîte de dialogue qui l'utilisent (cliquez pour y aller), et **Appliquer à** donne le style au widget sélectionné dans l'onglet Écrans.

Les anciens projets gardent leur apparence. Un style créé avec une version précédente affiche une note proposant de **convertir** son fond en fonds par état ; la conversion garde chaque état identique.

### Boutons image et images cliquables

Pour faire un bouton en images, donnez à son style (ou aux **changements locaux** du bouton, en bas de ses propriétés) un fond image par état : une image pour normal, une pour le survol, une pour appuyé... Gardez le texte par-dessus, ou désactivez **Afficher le texte** pour un bouton image seule. Un widget **Image** devient cliquable dès que vous lui donnez des **actions au clic** ; son style peut alors lui donner aussi des fonds survolé et appuyé.

## La boîte de dialogue

L'onglet **Boîte de dialogue** stylise ce que le joueur voit pendant l'histoire, avec un aperçu en direct dessiné par le jeu lui-même (choisissez **Une réplique** ou **Des choix**, et tapez un personnage et une réplique d'exemple) :

- **Boîte de texte** : une position fixe ou un placement ancré à l'écran (en bas, en haut, au centre... plus un décalage), sa taille, sa marge intérieure, le décalage du texte, la vitesse du texte, et un **style de la boîte** pour son fond (image ou 9-slice compris), sa bordure, son ombre, sa police et la couleur du texte.
- **Nom du personnage** : taille et style, dans la couleur du personnage ou celle du style, éventuellement sur une **plaque** avec son propre style, décalage et taille (une largeur de 0 s'ajuste au nom).
- **Indicateur « continuer »** : sa taille, sa couleur ou son image, et s'il clignote.
- **Choix** : taille des boutons, espacement, placement, numérotation, assombrissement de l'écran, et un **style des boutons** dont l'aspect survolé marque le choix sélectionné.
- **Menu rapide** : placement, style et largeur des boutons, espacement, ou le masquer.

Le bouton **+** à côté d'un champ de style crée un style à partir des couleurs actuelles et le relie ; le crayon l'ouvre dans l'onglet Thème. Un projet créé avant ces options garde exactement la même boîte de dialogue.

### Bulles de bande dessinée

Sous **Affichage des dialogues**, choisissez le **Mode d'affichage** : **Boîte classique** (par défaut) ou **Bulles de BD**. En mode bulles, les répliques des personnages apparaissent dans des bulles placées près du visage de celui qui parle, avec une pointe vers la tête ; le narrateur garde la boîte. L'aperçu de l'onglet montre le résultat.

- Une réplique est une bulle de parole par défaut. Commencez-la par `{bubble=thought}` pour un nuage de pensée ou `{bubble=shout}` pour un cri (une bulle en éclat).
- Réglages des **Bulles** : couleurs du fond et du contour, épaisseur du contour, arrondi des coins, largeur minimale et maximale, police, affichage et taille du nom du personnage, longueur et largeur de la pointe, écart avec la tête, marge de l'écran, animation d'apparition et contour dans la couleur du personnage.
- **Narrateur** : garder la boîte classique, ou afficher un **Cartouche** (en haut à gauche) avec sa propre couleur.
- **Personnage hors écran** : une bulle sans pointe, ou la boîte classique.
- **Par personnage** : couleurs de la bulle et position de la tête (décalage depuis le haut de l'image du personnage), pour que la pointe vise le bon endroit.

## Musique dans les menus

Un écran peut jouer sa propre musique. Dans l'onglet **Écrans**, sélectionnez l'écran (la racine de son arbre) et cochez **Musique de l'écran** dans ses propriétés :

- **Fichier audio** : un fichier musique ou son du projet. Un avertissement s'affiche si le fichier n'existe pas. La piste est empaquetée dans le jeu exporté.
- **Volume**, **Fondu entrée / sortie** (millisecondes) et **Jouer la piste en boucle**.
- **Continuer si l'écran suivant a la même piste** : ouvrir les options depuis le menu principal avec la même piste ne la relance pas. Décochez pour relancer la piste sur cet écran.
- **Par-dessus le jeu** : quand l'écran s'ouvre pendant la partie (pause, sauvegarde, chargement...), choisissez **Remplacer la musique du jeu** (la piste du menu joue ; **Remettre la musique du jeu à la fermeture** relance la piste en cours de l'histoire quand les menus se ferment) ou **Baisser la musique du jeu** (la piste du jeu continue au volume choisi et l'écran ne joue pas de piste).

La musique de l'écran choisi comme **menu principal** joue dès l'écran de démarrage. Lancer ou charger une partie la fait disparaître en fondu. Quand plusieurs écrans sont ouverts, le plus haut qui a une musique décide ; un écran sans musique laisse jouer celui du dessous. Revenir au titre relance la musique du menu principal.

Les boutons ont deux actions de plus : **Jouer une musique** (un fichier, en boucle, qui remplace la musique ; elle dure jusqu'à ce qu'un autre écran avec musique s'affiche) et **Arrêter la musique**. Dans l'éditeur, l'aperçu en direct joue la musique de l'écran affiché ; le bouton muet de l'aperçu la coupe.

## Emplacements de sauvegarde et de chargement

Le widget **Emplacements** affiche les sauvegardes du jeu en grille de cartes, avec des onglets de catégories, des boutons de page et un bouton Supprimer sous chaque carte. Par défaut il utilise la carte standard (miniature, nom de l'emplacement, date et dernière réplique). Tout peut être restylé :

- **Apparence des emplacements** (dans ses propriétés) : un **style** pour les cartes, les cartes vides, les cartes incompatibles, les onglets, les boutons de page, le bouton Supprimer, la boîte de confirmation et ses boutons Oui / Non ; la **hauteur des cartes** et les **espaces** entre colonnes et lignes, à côté des **colonnes** et **lignes**. Les apparences survol, appui, focus et désactivé viennent de ces styles (voir *Les styles*).
- **Personnaliser la carte** écrit la carte standard sous forme de widgets : les enfants du widget Emplacements sont l'intérieur d'une carte, répété pour chaque emplacement. Modifiez-les comme n'importe quels widgets : déplacer la miniature, ajouter le temps de jeu, changer polices et couleurs. L'apparence ne change pas tant que vous ne modifiez rien. **Revenir à la carte par défaut** les supprime.
- Dans la carte, liez un **texte** à un **champ de l'emplacement** : titre (Emplacement 3), numéro, date et chapitre, date et heure, temps de jeu, chapitre (scène), dernière réplique, état (Vide / Incompatible) ; liez une **Image** à la **miniature**. Les conditions **l'emplacement est vide**, **l'emplacement contient une sauvegarde** et **la sauvegarde est incompatible** n'affichent un widget que dans un état (par exemple un texte « Vide », ou un cadenas sur les sauvegardes incompatibles).
- Le temps de jeu et le chapitre sont vides pour une sauvegarde qui ne les enregistre pas.

Un projet créé avant cette version garde ses cartes exactement comme avant.

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

**Quand les lignes ne tiennent pas.** Sous **Si les lignes ne tiennent pas**, choisissez le comportement d'une longue liste : **Défilement** (molette, glisser, clavier et manette, avec une barre), **Pages** (boutons Précédent / Suivant et compteur de page) ou **Rien (peut déborder)**. Donnez à la liste une hauteur *Remplir* ou *Fixe* pour qu'elle connaisse la place disponible. Les écrans Galerie, Succès et Rejouer du jeu de base défilent par ligne, ou se divisent en pages, quand leurs cartes ne tiennent plus à l'écran. Un conteneur **Liste défilante** fait de même pour n'importe quel groupe de widgets.

### Les transitions d'écran

Chaque écran peut avoir une **transition d'ouverture** et une **transition de fermeture** : fondu, glissement vers le haut, le bas, la gauche ou la droite, ou mise à l'échelle, avec une durée et une courbe d'accélération (les courbes nommées de votre projet sont acceptées). Les transitions sont purement visuelles : l'écran est ouvert ou fermé immédiatement, seul son dessin est animé. Le retour en arrière, le chargement d'une sauvegarde et la préférence d'accessibilité **réduire les animations** les coupent.

### L'exemple de la démo

L'histoire de démonstration (`examples/stories`) utilise les deux sortes d'écran :

- Quand Alice quitte la boutique, une étape **Ouvrir un écran** avec **Attendre** ouvre l'écran `inventory` avec le paramètre `owner` réglé sur « Alice ». L'écran répète la liste `inventory` ; chaque bouton ferme l'écran avec son objet, qui atterrit dans la variable `used_item`. Le bouton **Retour** et Échap ferment sans valeur. Il glisse vers le haut à l'ouverture et disparaît en fondu à la fermeture.
- Au début du jeu, une étape **Ouvrir un écran** sans **Attendre** ouvre le HUD `status` : un petit panneau en haut à droite qui affiche la variable `credits`, mise à jour au fil de l'histoire.

## Galerie et rejouer les scènes

L'interface standard compte deux écrans de plus, ouverts depuis le menu principal : **Galerie** et **Rejouer**.

* **Ce qui les débloque.** Toute image montrée en fond pendant une partie est débloquée dans la galerie, et toute scène où le joueur entre est « vue » et peut être rejouée. Cette progression est conservée d'une partie à l'autre et survit à la suppression des sauvegardes (elle est rangée avec les variables persistantes). L'aperçu de l'éditeur ne l'enregistre jamais.
* **Galerie.** Les images débloquées sont affichées en vignettes, les autres sous forme de carte « Verrouillée ».
* **Rejouer.** Les scènes vues sont listées dans l'ordre de l'histoire ; en choisir une joue cette scène seule. Elle démarre avec les variables qu'avait la scène la première fois que le joueur y est entré (les valeurs initiales déclarées si on ne les connaît pas), et rien de ce qu'elle fait n'est conservé : ni sauvegarde, sauvegarde rapide ou automatique, ni changement des variables persistantes ou des déblocages. À la fin de la scène (ou si le joueur revient au menu principal), le jeu revient à l'écran Rejouer. Une scène *appelée* par l'histoire s'exécute normalement ; un saut vers une autre scène termine le rejouer.

### Choisir ce que proposent la galerie et le menu Rejouer

Dans l'éditeur **Interface**, sélectionner la liste générée de l'écran `gallery` (ou `replay`). Sa **Liste** peut être **Images de la galerie** ou **Scènes à rejouer** ; cocher les images ou scènes proposées (aucune cochée = tous les fonds de l'histoire / toutes les scènes). Ce choix est rangé dans l'écran, pas dans l'histoire.

Pour composer ses propres écrans : dans une liste générée, une **Image** avec **Image de la ligne** affiche l'image de la ligne ; la condition **l'élément de la ligne est renseigné** distingue une image débloquée d'une image verrouillée ; un bouton avec l'action **Rejouer une scène** (case **Scène de la ligne** cochée) rejoue la scène de la ligne. « Rejouer une scène » peut aussi viser une scène fixe. Un rejouer ne démarre que depuis l'écran titre.


## Succès

Les succès se définissent dans le panneau **Succès** : un **titre**, une **description**, une **icône** facultative (une image du projet), une **icône verrouillée** facultative (affichée à la place de l'icône tant que le succès n'est pas débloqué ; sans elle, l'icône est utilisée) et la case **Caché tant qu'il n'est pas débloqué**. Un succès caché montre un texte neutre (titre, description et icône) dans la liste tant qu'il n'est pas débloqué. Titres et descriptions sont des textes ordinaires de l'histoire : ils se traduisent dans le panneau **Traductions** comme les autres.

On en débloque un depuis l'histoire avec l'étape **Débloquer un succès** (groupe Logique), ou avec le bouton *Ajouter un nœud de déblocage ici* du panneau. La première fois que l'histoire atteint l'étape dans une vraie partie, le succès est débloqué pour de bon et une **notification** l'annonce (« Succès débloqué : Premiers mots »). Ensuite l'étape ne fait plus rien. En particulier :

* revenir en arrière puis rejouer la réplique ne réaffiche **pas** la notification (un déblocage ne fait pas partie de la partie sauvegardée : il n'est jamais repris) ;
* un **rejouer de scène** ne débloque jamais rien et n'écrit jamais rien ;
* les succès débloqués sont conservés avec les variables persistantes (`persistent.ron`) : ils survivent à la suppression des sauvegardes et à une nouvelle partie. L'aperçu de l'éditeur ne les enregistre jamais.

L'interface standard gagne un écran **Succès**, ouvert depuis le menu principal (le bouton n'apparaît que si l'histoire définit des succès) : une ligne de progression (« 3 / 10 ») et une carte par succès, avec son icône, son titre, sa description et la mention « Verrouillé » tant qu'il ne l'est pas.

Pour composer son propre écran, utiliser dans l'éditeur **Interface** une liste générée dont la **Liste** est **Succès**. Dans une ligne, lier un texte à **Succès de la ligne** (titre, description) ou une **Image** à son **icône** ; la condition **le succès de la ligne est débloqué** distingue les succès obtenus ; **progression (3 / 10)** marche partout, et la condition **l'histoire a des succès** cache un bouton quand il n'y en a pas.

Les plateformes (Steam, itch.io, consoles) passent par le trait `AchievementHook` d'`orbit_runtime` (`RuntimeConfig::achievement_hook`) : il reçoit l'identifiant stable du succès quand il est débloqué, et une fois au démarrage pour ceux que le joueur a déjà. Aucune plateforme n'est branchée pour l'instant (Steam est au backlog) ; sans hook, seules les données persistantes sont écrites.

## Notifications

L'étape **Notification** (groupe Dialogue du menu d'insertion) affiche un court message dans un coin de l'écran, une *notification*, et l'histoire continue aussitôt : elle n'attend jamais la disparition de la notification. Ses champs :

* **Texte** : le balisage est permis, et `[variable]` est remplacé par la valeur de la variable au moment de l'affichage. Le texte se traduit comme tous les autres textes de l'histoire.
* **Icône** (facultatif) : une image affichée à gauche du texte. **Son** (facultatif) : joué avec la notification. **Durée** (facultatif, en secondes) : le temps d'affichage. **Style** (facultatif) : le nom d'un style de notification du thème ; sans lui, le style `default` est utilisé.

En format texte : `notify "Sauvegardé !" [icon x] [sound y] [style z] [duration n]`. Dans un script Rhai : `notify("texte")`, `notify("texte", "style")` ou `notify("texte", #{style: "system", icon: "cloche", sound: "ping", duration: 4.0})`. Un bouton peut aussi la déclencher avec l'action **Notify** (un texte, et un style si besoin).

Les succès utilisent le style `achievement`, et les messages du moteur lui-même le style `system` : modifiez leur apparence dans le thème comme n'importe quel autre style.

Les notifications ne sont jamais sauvegardées. Un retour en arrière, ou le chargement d'une sauvegarde, retire celles de l'histoire qui sont à l'écran. En mode **Passer**, une notification suit le réglage **Passer** ci-dessous.

### Le thème : `notifications`

L'apparence se règle dans l'éditeur d'**Interface**, onglet **Notifications** :

* **Ancrage** (en haut à droite par défaut) et **Décalage** depuis ce coin ; **Largeur** (0 s'ajuste au texte) ; **Maximum visibles** (les autres attendent dans une file) ; **Espace** entre deux notifications et **Sens d'empilement** (Auto, Vers le bas ou Vers le haut).
* **Durée** et **Animation** : Glissement, Fondu ou Aucune, avec sa durée en secondes. Quand le joueur demande moins de mouvement, le glissement devient un fondu. **Son par défaut**, et **Clic pour fermer**.
* **Comportement en mode Passer** : Afficher, Bref (un affichage plus court) ou Supprimer (rien n'est affiché pendant qu'on passe).
* **Styles nommés** : chacun a un fond (une couleur, ou un style du thème comme une image à neuf zones), une bordure, un rayon d'angle, une marge intérieure, une taille d'icône et une icône par défaut, la taille, la couleur et la police du texte, et éventuellement sa propre largeur, durée et son.

L'aperçu propose un bouton **Afficher une notification de test** pour voir le résultat sans jouer l'histoire.
