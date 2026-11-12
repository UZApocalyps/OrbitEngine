# Tutoriel : des personnages expressifs

Deux façons de donner vie à vos personnages. La première change la manière dont leurs paroles s'affichent : des bulles de BD qui pointent vers celui qui parle. La seconde change leur façon de bouger : un squelette d'os permet à un dessin de respirer, de cligner des yeux et de saluer. Les deux sont indépendantes, et vont bien ensemble.

## Partie 1. Dialogues façon BD avec des bulles

### Objectif

Les répliques des personnages apparaissent dans des bulles à côté de leur tête, chaque personnage avec ses propres couleurs. Le narrateur garde un cartouche, et une réplique peut devenir un nuage de pensée ou un cri.

### Avant de commencer

- Un projet avec au moins un personnage affiché en scène (une étape **Afficher un personnage** avant ses répliques). La bulle pointe vers la tête du personnage.
- L'interface personnalisée doit être activée : **Outils > Éditeur d'interface**, puis **Activer l'interface personnalisée** si le panneau dit qu'elle est désactivée (voir *Tutoriel : personnaliser l'interface du jeu*).

### Étapes

1. Dans le panneau **Interface**, ouvrez l'onglet **Boîte de dialogue**. Trouvez **Affichage des dialogues** et réglez le **Mode d'affichage** sur **Bulles de BD**. Les répliques des personnages apparaissent désormais dans des bulles ; le narrateur garde la boîte.
2. Dans la section **Bulles**, ajustez l'aspect général avec l'aperçu : couleurs **Fond / contour**, **Épaisseur du contour**, **Arrondi des coins**, **Largeur min / max**, **Police**, et **Apparition** pour l'animation d'entrée.
3. Utilisez **Afficher le nom du personnage** et **Taille du nom** si vous voulez le nom écrit dans la bulle. Cochez **Contour dans la couleur du personnage** pour teinter chaque bulle de la couleur de celui qui parle.
4. Réglez la **Pointe : longueur / largeur** et l'**Écart avec la tête**. Gardez une **Marge de l'écran** pour que les bulles restent dans l'image.
5. Sous **Narrateur**, choisissez l'affichage de la narration : **Boîte classique** ou **Cartouche (en haut à gauche)**, avec sa **Couleur du cartouche**.
6. Sous **Personnage hors écran**, choisissez **Bulle sans pointe** (la réplique reste dans une bulle qui ne désigne personne) ou **Boîte classique**.
7. Dans **Par personnage**, donnez à chaque personnage son **Fond** et son **Contour**, ainsi qu'un **Décalage de la tête** : jusqu'où la pointe doit viser depuis le haut de l'image. Un grand sprite avec un chapeau demande un décalage plus important qu'un petit. Réglez-le avec l'aperçu jusqu'à ce que la pointe touche le visage.
8. Dans l'histoire, mettez `{bubble=thought}` dans une réplique (le plus souvent au début) pour dessiner un nuage de pensée, ou `{bubble=shout}` pour une bulle de cri en dents de scie. Par exemple `{bubble=shout}Sors d'ici !`. La balise ne s'affiche pas.
9. Appuyez sur **Lecture** (F6) dans Preview, ou sélectionnez une étape et appuyez sur **Ici** (F7), pour voir le vrai résultat.

### Résultat

Chaque personnage parle avec ses couleurs, les bulles pointent vers les têtes, le narrateur utilise un cartouche et les balises donnent des pensées et des cris.

### Pour aller plus loin

- Déplacez les personnages avec le panneau **Éditeur de vue**, ou donnez-leur d'autres positions : les bulles suivent le personnage.
- Combinez avec l'étape **Effet** (une secousse sur une réplique `{bubble=shout}`) pour le rythme comique.
- Revenez à la **Boîte classique** quand vous voulez : rien ne change dans l'histoire.

### En cas de problème

- **La pointe vise au mauvais endroit** : ajustez le **Décalage de la tête** de ce personnage.
- **Pas de pointe du tout** : le personnage n'est pas en scène à ce moment-là, c'est donc le réglage **Personnage hors écran** qui s'applique.
- **La balise s'affiche comme du texte** : vérifiez la syntaxe : accolades, pas d'espace, et l'une des trois formes connues (`{bubble=speech}`, `{bubble=thought}` ou `{bubble=shout}`). Toute autre valeur reste du texte ordinaire.
- **La bulle déborde de l'écran** : augmentez la **Marge de l'écran** ou réduisez le maximum de **Largeur min / max**.

## Partie 2. Personnages animés avec un rig

### Objectif

Transformer un dessin en personnage qui respire, puis l'utiliser dans votre histoire comme n'importe quelle expression. Comptez une vingtaine de minutes la première fois.

### Avant de commencer

- Un PNG de votre personnage à fond transparent, idéalement découpé comme une poupée de papier : tête, torse, bras en images séparées si vous voulez qu'ils bougent indépendamment. Ou un fichier à calques (PSD, Krita `.kra` ou OpenRaster `.ora`) avec un calque par partie.
- Un projet ouvert dans l'éditeur, avec le personnage déjà créé dans le panneau **Personnages**.

### Étapes

1. Choisissez **Outils > Rig > Nouveau rig depuis une image…** et sélectionnez votre PNG. L'image est copiée dans le projet et l'éditeur de rig s'ouvre : **Vue du rig**, **Hiérarchie du rig**, **Inspecteur du rig**, **Clips du rig** et **Timeline du rig**. Une barre d'étapes indique où vous en êtes : **Importer l'image**, **Dessiner les os**, **Lier**, **Poser / animer**.
2. Si vous partez d'un fichier à calques, choisissez plutôt **Outils > Rig > Nouveau rig depuis un fichier à calques…**. Chaque calque visible devient une partie à sa position exacte. Quand l'éditeur demande s'il faut créer un os par groupe de calques, répondez oui si vos groupes ont un sens (un groupe bras, un groupe tête), non sinon. Ajoutez d'autres images avec **Ajouter une partie (image)…** ou déposez-les sur la vue.
3. **Dessinez les os.** Appuyez sur **B** et tirez sur l'image d'une articulation à l'autre : hanches vers cou, épaule vers coude, coude vers main. Un nouvel os se rattache à l'os sélectionné ; cliquez sur un endroit vide pour démarrer une nouvelle chaîne.
4. Cliquez sur **Lier aux os**. Chaque partie suit l'os le plus proche, adouci entre les os. Pour affiner, appuyez sur **P** pour le pinceau de poids et peignez l'influence de l'os sélectionné.
5. La vue est maintenant en mode pose (**Tab** bascule avec la pose de repos). Sélectionnez un os et tirez son anneau : l'image se plie. Poser ne modifie pas le rig lui-même.
6. Cliquez sur **Ajouter au projet**. Le rig est enregistré dans `assets/rigs/<nom>.rig.ron` et apparaît dans la bibliothèque.
7. Ouvrez le panneau **Clips du rig** et cliquez sur **Nouveau clip**. Nommez-le `idle`, réglez sa **Durée** (3 secondes conviennent bien) et sa **Boucle** sur **Sans fin**.
8. Dans la **Timeline du rig**, placez la tête de lecture à 0 s et cliquez sur **Clé de pose** (K) pour enregistrer la pose de repos. Avancez à 1,5 s, tournez l'os du torse ou de la tête de quelques degrés en mode pose et cliquez de nouveau sur **Clé de pose**. Revenez à 3 s et enregistrez une troisième fois la pose de repos. Vous pouvez aussi ajouter à la main des entrées **Piste** et **Clé**, ou activer la clé auto pour que poser un os enregistre des clés tout seul.
9. Lancez la lecture dans la vue pour voir la boucle. Enregistrez de nouveau le rig avec **Ajouter au projet** quand vous êtes satisfait.
10. Ouvrez le panneau **Personnages**, sélectionnez le personnage et choisissez une expression (ou **Ajouter une expression**). Réglez son type sur **Rig**, choisissez le rig, le **Skin** et le **Clip** `idle`, ainsi que ce qui se passe à la fin du clip (**Boucle**, **Une seule fois** ou **Aller-retour**). L'image de l'expression reste l'affiche affichée pendant le chargement du rig.
11. Dans l'histoire, utilisez **Afficher un personnage** avec cette expression, comme d'habitude. Le personnage respire désormais en scène. Afficher le même personnage avec une autre expression du même rig fait un fondu enchaîné de 0,2 seconde entre les deux clips.

### Résultat

Un personnage qui bouge dans le jeu, avec un dessin que vous pouvez toujours retoucher, et des clips réutilisables pour d'autres expressions (un salut, un hochement de tête) du même rig.

### Pour aller plus loin

- Faites **cligner** le personnage : ajoutez une piste **Attachment** sur une paupière, avec des clés qui l'affichent et la cachent (voir *Personnages et animations*).
- Réutilisez un squelette pour plusieurs personnages avec **Dupliquer le rig…** et **Copier des clips d'un autre rig…** (menu Outils ou menu clé à molette de l'éditeur de rig).
- Découpez une image à plat en calques (cheveux, visage, yeux) avec l'outil de découpe, et peignez les retouches directement sur la partie.
- Si le personnage apparaît au mauvais endroit sur scène, utilisez **Recentrer sur l'origine…**.

### En cas de problème

- **L'image ne bouge pas quand je pose un os** : liez d'abord (**Lier aux os**). Si vous ajoutez des os plus tard, liez de nouveau.
- **L'expression affiche une image fixe** : le panneau Problèmes en donne la raison, par exemple un rig sans clip, un clip renommé, ou une image qui n'est pas un fichier du projet.
- **Le personnage est déchiqueté** : le maillage est tracé d'après la transparence ; nettoyez les pixels isolés de l'image, ou utilisez l'outil maillage (M) pour l'ajuster.
- **Tout bouge quand je bouge un seul os** : les os doivent former des chaînes avec les bons parents. Dessinez une nouvelle chaîne depuis un endroit vide plutôt que depuis l'os sélectionné.
- **J'ai modifié une image et un autre personnage a changé aussi** : les images peuvent être partagées entre rigs ; l'éditeur liste ce qui utilise aussi un fichier avant la première modification de pixel et propose d'en faire une copie.
