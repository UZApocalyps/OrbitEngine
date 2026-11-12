# Premiers pas

OrbitEngine permet de créer un visual novel sans écrire de code. Vous écrivez des répliques, proposez des choix, affichez des personnages et des décors, et l'éditeur joue le résultat au fur et à mesure.

## Créer votre premier projet

1. Ouvrez l'éditeur. La page d'accueil propose **Nouveau projet** (Ctrl+N).
2. Saisissez un nom et choisissez le dossier qui contiendra votre projet.
3. Choisissez un **modèle** :
   - **Vide** : une scène et deux répliques, pour partir de zéro.
   - **Démarrage visual novel** : deux scènes, deux personnages, un choix, une variable, des écrans de menu personnalisés et un splash. Vous pouvez y jouer tout de suite, puis le réécrire.
4. Appuyez sur **Créer**. Le projet s'ouvre.

Un projet est un dossier ordinaire. Votre histoire est le fichier `story.orbit`, vos images et sons sont dans `assets/`, vos écrans personnalisés dans `ui/`. Vous pouvez mettre tout le dossier sous gestion de versions ou le copier sur un autre ordinateur.

## Les fenêtres principales

- **Scène** : l'histoire sous forme de liste d'étapes, dans l'ordre de lecture. C'est là que vous écrivez la plupart du temps.
- **Graphe** : la même histoire dessinée en boîtes et en flèches, pratique pour voir où mènent les choix.
- **Inspecteur** : tous les réglages de l'étape sélectionnée.
- **Personnages** et **Variables** : les définitions utilisées par votre histoire.
- **Bibliothèque** : vos images et sons. Glissez des fichiers dans la fenêtre pour les ajouter.
- **Preview** : joue l'histoire avec le vrai moteur, et se recharge quand vous modifiez.
- **Problèmes** : vérifications en direct. Une erreur bloque l'export, un avertissement non. Les vérifications tournent en arrière-plan environ 300 ms après votre dernière modification. Désactivez-les avec **Affichage > Validation automatique** (utile sur les très gros projets) : la liste est alors estompée quand elle est périmée et se met à jour avec **Valider** (Ctrl+F7). L'export et la compilation valident toujours d'abord.

Chaque fenêtre peut être rouverte depuis le menu **Affichage**, et réorganisée en faisant glisser son onglet.

## Enregistrer et annuler

Ctrl+S enregistre. Ctrl+Z annule et Ctrl+Y rétablit, aussi loin que nécessaire. Si l'éditeur se ferme brutalement, il propose de restaurer votre travail non enregistré à la réouverture du projet.

## Et ensuite

Chaque chapitre de ce manuel traite d'un sujet. Ouvrez **Aide > Manuel** à tout moment (ou appuyez sur F1). Le manuel suit la langue de l'éditeur.

## Les menus

- **Fichier** : Nouveau projet, Ouvrir, Enregistrer, Fermer, **Import, export et traductions** (import Ren'Py, export du jeu, import et export des traductions), Importer des ressources.
- **Édition** : Annuler / Rétablir, **Insérer** (tous les types d'étapes, en sous-menus par catégorie : Dialogue, Scène, Effets, Écrans, Son, Logique, Navigation, 3D ; plus l'assistant *Nouvelle carte de navigation (maison)*), **Blocs** et **Scènes** (dupliquer, déplacer, renommer...), Aller au problème suivant.
- **Affichage** : **Panneaux** (groupés en Histoire, Édition, Assets, Outils, Diagnostic, Autres), **Disposition** (dispositions enregistrées et espaces de travail des outils), **Options** (Validation automatique, Fonctionnalités 3D expérimentales...) et **Langue**.
- **Outils** : l'Assistant (MCP), **Extensions > Plugins...** (plugins de l'éditeur, voir le chapitre *Plugins de l'éditeur*), le Studio IA quand son plugin est installé, les outils de rig comme *Nouveau rig depuis une image...*.
- **Preview** et **Aide** (le manuel, F1).

Un clic droit sur une ligne du panneau Scène, un bloc du Graphe, une entrée de l'Explorateur, un problème ou la scène, puis **Modifier**, ouvre directement ses réglages dans l'Inspecteur. La **palette de commandes** (Ctrl+K) retrouve n'importe quelle action par son nom.

## Raccourcis clavier

**Projet**

- Ctrl+N nouveau projet, Ctrl+O ouvrir un projet, Ctrl+Maj+O ouvrir un fichier d'histoire, Ctrl+S enregistrer, Ctrl+W fermer le projet, Ctrl+Q quitter.
- Ctrl+Z annuler, Ctrl+Y (ou Ctrl+Maj+Z) rétablir.
- Ctrl+K palette de commandes, Ctrl+Maj+F rechercher dans tout le projet, Ctrl+Maj+I importer des ressources, Ctrl+Maj+A Assistant, F1 manuel.
- Ctrl+F7 valider, F8 aller au problème suivant.

**Preview**

- F6 lecture / pause, Maj+F6 arrêter, Ctrl+F6 recommencer, F7 jouer depuis l'étape sélectionnée.

**Éditeur de scène**

- Ctrl+Entrée ajouter une réplique, Ctrl+I insérer une étape (menu rapide), Suppr supprime la sélection.
- Ctrl+C, Ctrl+X, Ctrl+V copier, couper, coller des lignes ; Ctrl+D dupliquer ; Ctrl+A tout sélectionner ; Alt+Haut / Alt+Bas déplacer la sélection ; F2 renommer la scène.
- Dans la **Vue texte**, Ctrl+Entrée applique le texte.

**Éditeur de rig**

- B outil os, M outil maillage, P pinceau de poids, W / E / R déplacer, tourner, mettre à l'échelle, F cadrer tout, Tab mode repos / pose, K enregistrer une clé en mode pose, Suppr supprimer l'os sélectionné, [ et ] taille du pinceau, Alt-clic source du tampon ou pipette.

**Éditeur de vue, zones cliquables**

- Suppr supprime la zone sélectionnée ; pendant le dessin d'un polygone, Entrée le ferme, Retour arrière retire le dernier point, Échap annule.

**Vue 3D** (expérimental)

- W / E / R déplacer, tourner, mettre à l'échelle, F cadrer la sélection, Suppr supprimer, Ctrl+D dupliquer, 1 / 3 / 7 vue de face, de droite, de dessus (avec Ctrl : le côté opposé), G poser la sélection au sol, P mode pose d'un personnage, Échap quitter le mode pose.

**Dans le jeu**

- F11 plein écran. Sur une carte point & click, V affiche toutes les zones ; le clavier (chiffres compris) et la manette passent d'une zone à l'autre. L'action *afficher les zones* de la manette est Select / Vue par défaut.

## Dépannage et FAQ

**Le jeu web exporté reste blanc ou bloqué quand j'ouvre `index.html`.** Les navigateurs refusent WebAssembly et le chargement de fichiers sur les pages `file://`. Servez le dossier en http : utilisez **Tester dans le navigateur** dans le panneau Export, ou `orbit export web --project mon_jeu --out dist --serve`, ou `cargo xtask serve --dir dist` (il sert sur `http://127.0.0.1:8080`, `--port` le change). Pour publier, mettez le dossier sur n'importe quel hébergeur statique.

**Il n'y a ni menu, ni panneau, ni chapitre du manuel pour la 3D.** Les fonctions 3D sont expérimentales et masquées par défaut. Activez-les avec **Affichage > Options > Fonctionnalités 3D expérimentales** (ou lancez l'éditeur avec la variable d'environnement `ORBIT_EXPERIMENTAL_3D=1`). Le chapitre *Scènes 3D* de ce manuel n'apparaît qu'une fois la 3D activée. Les projets qui utilisent déjà la 3D continuent de fonctionner interrupteur éteint.

**Un plugin de l'éditeur est « Incompatible » après une mise à jour de l'éditeur.** Un plugin est construit pour une version exacte de l'éditeur. Recompilez-le pour la nouvelle version (`cargo xtask plugins`), ou installez la version faite pour elle. Si un plugin empêche l'éditeur de démarrer correctement, lancez-le avec `--no-plugins`, puis désactivez ou désinstallez le plugin dans **Outils > Extensions > Plugins...**.

**Le Studio IA n'arrive pas à joindre mon serveur local.** `localhost` peut désigner l'IPv6 (`::1`) alors que le serveur n'écoute qu'en IPv4 : essayez `http://127.0.0.1:11434/v1` (Ollama) ou `http://127.0.0.1:8188` (ComfyUI). L'URL d'un modèle de texte se termine par `/v1`.

**Ollama répond « model not found ».** Le modèle n'est pas encore téléchargé : lancez `ollama pull <modèle>` dans un terminal (par exemple `ollama pull llama3.1`), puis refaites **Tester la connexion / lister les modèles**. Le nom du modèle doit être exact.

**Un modèle 3D manque dans la preview ou dans le jeu.** Le fichier du modèle doit se trouver dans le dossier `models` à côté du fichier de la scène. Le panneau Problèmes et le journal d'export nomment le modèle manquant. Ouvrez la scène dans l'éditeur 3D et réimportez le modèle, ou copiez le fichier à cet endroit. Si l'histoire montre une copie plus ancienne d'une scène que vous modifiez, utilisez *Utiliser le fichier de l'histoire* dans la vue 3D (voir le chapitre *Scènes 3D*).

**Les personnages apparaissent au mauvais endroit de la scène.** L'origine d'un rig est l'endroit où se place le bas-centre du personnage : utilisez **Recentrer sur l'origine...** dans l'éditeur de rig.

**Des caractères de ma traduction s'affichent en carrés vides.** La police par défaut ne couvre que les écritures latines : ajoutez une police pour cette langue dans l'éditeur d'interface (voir *Traductions*).

**Anciens projets, sauvegardes et jeux.** Les anciens projets et sauvegardes se chargent toujours dans les versions récentes. Un jeu exporté avec un moteur plus récent peut être refusé par un player plus ancien (pack « trop récent ») : réexportez-le avec le player correspondant.
