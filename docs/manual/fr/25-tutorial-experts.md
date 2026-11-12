# Tutoriel : scènes 3D et plugins d'éditeur

Deux tutoriels pour aller plus loin. Le premier place une scène 3D derrière vos personnages 2D, avec des caméras et un personnage 3D posé. Le second montre comment étendre l'éditeur lui-même avec un plugin à vous.

## Partie 1. Une scène 3D dans votre histoire (expérimental)

### Objectif

Construire une petite pièce en 3D avec une lumière, deux caméras et un personnage posé, puis l'afficher dans l'histoire avec une coupe d'un plan large vers un gros plan.

### Avant de commencer

- Un projet avec une scène où la 3D sera affichée.
- Un modèle 3D, par exemple un personnage dans un fichier `.glb` (ou un fichier `.gltf` avec ses fichiers `.bin` et textures à côté). Un modèle avec squelette permet de le poser. Sans modèle, vous pouvez suivre le tutoriel avec les formes intégrées.
- Un ordinateur avec une carte graphique : l'éditeur 3D et l'aperçu en ont besoin.

### Étapes

1. **Activer la fonctionnalité.** La 3D est expérimentale et masquée par défaut. Choisissez **Affichage > Options > Fonctionnalités 3D expérimentales** pour la cocher. Les étapes 3D apparaissent alors dans le menu d'insertion.
2. **Ouvrir l'éditeur 3D.** Choisissez **Outils > Éditeur de scènes 3D**. Trois panneaux s'ouvrent : la **Vue 3D**, la **Hiérarchie 3D** et l'**Inspecteur 3D**. Dans la vue, le glisser-clic droit fait orbiter, le glisser-clic milieu déplace et la molette zoome.
3. **Créer la scène.** Cliquez sur **Nouvelle scène 3D**. Elle démarre avec un soleil et une caméra nommée `main`. Cliquez dès maintenant sur **Enregistrer la scène 3D** et choisissez un nom : les modèles importés sont rangés dans un dossier `models` à côté du fichier de la scène, elle doit donc être enregistrée d'abord.
4. **Construire la pièce.** Dans la hiérarchie, cliquez sur **Ajouter**, puis **Forme**, et ajoutez un **Plan** pour le sol, agrandi dans l'inspecteur (**Taille**), et un **Cube** pour un meuble. Une nouvelle forme se pose sur le sol avec le matériau **Default** de la scène ; changez sa **Couleur de base** dans l'inspecteur. Appuyez sur W, E ou R pour déplacer, pivoter ou redimensionner l'objet sélectionné, et sur G pour le poser sur le sol.
5. **L'éclairer.** Ajoutez une lumière ponctuelle ou un projecteur depuis **Ajouter**, ou gardez le soleil. Réglez son **Intensité** et sa **Couleur**. Quand rien n'est sélectionné, l'inspecteur montre l'**Environnement** : couleur de fond ou **Skybox**, lumière ambiante et brouillard.
6. **Importer votre personnage.** Cliquez sur **Importer un glTF** et choisissez le fichier `.glb`. Le modèle apparaît dans la scène. Placez-le avec le manipulateur de déplacement, ou glissez-le depuis la liste **Modèles** dans la vue.
7. **Régler les caméras.** Sélectionnez la caméra `main` et donnez-lui un bon nom dans **Nom de la caméra**, par exemple `large`. Cliquez sur **Regarder par cette caméra** pour voir son cadrage ; faites orbiter la vue jusqu'à un bel angle, puis cliquez sur **Aligner sur la vue** pour copier cet angle dans la caméra. Ajoutez une seconde caméra depuis **Ajouter**, nommez-la `gros_plan` et cadrez le visage du personnage de la même façon.
8. **Poser le personnage.** Sélectionnez le modèle. S'il a un squelette, l'inspecteur a une section **Squelette**. Cliquez sur **Pose** (ou appuyez sur P) pour entrer en mode pose, cliquez sur un os et tirez ses anneaux pour le tourner (W sur la racine déplace tout le corps, Suppr réinitialise l'os, Échap quitte le mode). Saisissez un **Nom de la pose** (par exemple `bras_croises`) et cliquez sur **Enregistrer la pose** ; **Appliquer** ramène une pose enregistrée à tout moment, et **Réinitialiser la pose** revient à la pose de repos. Sous **Animations**, **Nouvelle animation** et **Clé de pose ici** enregistrent un court mouvement.
9. **L'ajouter au projet.** Cliquez sur **Ajouter au projet (ressource scène 3D)** dans la barre d'outils de la vue. La scène est enregistrée dans `assets/3d/<nom>.scene3d.ron` et apparaît dans la bibliothèque.
10. **L'afficher dans l'histoire.** Cliquez sur **Afficher la scène 3D dans l'histoire** : l'éditeur insère l'étape **Afficher une scène 3D** après la réplique sélectionnée et lance l'aperçu à cet endroit. Ou insérez vous-même l'étape depuis le groupe **3D** du menu d'insertion. Dans l'Inspecteur, choisissez la **Scène 3D**, la **Caméra** `large` et une **Transition**.
11. Insérez quelques répliques plus loin une étape **Caméra 3D** et choisissez `gros_plan` : l'image coupe vers le gros plan. À la fin, insérez une étape **Masquer la scène 3D** pour revenir à un décor 2D ordinaire.
12. Appuyez sur **Lecture** (F6) dans Preview. Vos personnages 2D et la boîte de dialogue sont dessinés par-dessus l'image 3D.

### Résultat

Une histoire qui passe d'un plan large à un gros plan sur un personnage 3D posé, dans une pièce que vous avez construite, avec le dialogue 2D par-dessus.

### Pour aller plus loin

- Utilisez la scène 3D comme décor de tout un chapitre et gardez les personnages 2D par-dessus.
- Dans le panneau **Export**, **Qualité 3D (ordinateur)** règle la qualité des jeux Windows et Linux ; les jeux web et Android utilisent toujours le profil web, plus léger. La section **Qualité 3D** compare lumières, triangles et textures au budget.
- Un assistant IA (MCP) peut lire et modifier les scènes 3D et prendre une image depuis n'importe quelle caméra (voir *Scènes 3D*).

### En cas de problème

- **Les étapes 3D ne sont pas dans le menu** : la fonctionnalité est désactivée. Cochez **Fonctionnalités 3D expérimentales** dans **Affichage > Options**.
- **« Enregistrez d'abord la scène 3D »** à l'import : enregistrez la scène pour que le modèle ait un dossier où être copié.
- **Le modèle paraît plat ou sombre** : ajoutez une lumière, ou augmentez la lumière ambiante dans l'**Environnement**.
- **Le panneau Problèmes cite une caméra** : une étape désigne une caméra que la scène n'a pas (renommée ou supprimée). Choisissez une autre caméra dans l'étape.
- **L'histoire montre une copie d'une ancienne scène** : l'éditeur l'indique quand le fichier de l'histoire diffère de celui que vous modifiez ; utilisez **Utiliser le fichier de l'histoire**.

## Partie 2. Créer un plugin d'éditeur

### Objectif

Construire, empaqueter et installer le plugin d'exemple `hello`, puis le modifier pour ajouter votre propre action de menu. Il vous faut Rust et les sources d'OrbitEngine.

### Avant de commencer

- La chaîne d'outils Rust du dépôt (`rust-toolchain.toml`) et les sources d'OrbitEngine, avec l'éditeur construit depuis elles : un plugin ne se charge que dans un éditeur construit depuis les mêmes sources, par le même compilateur et avec le même profil.
- Quelques connaissances de Rust. Le chapitre *Plugins de l'éditeur* explique les notions et les règles de compatibilité en détail.

### Étapes

1. **Regarder le modèle.** Ouvrez le dossier `examples/editor_plugins/hello` : un `Cargo.toml` (une `cdylib` qui ne dépend que de `orbit_editor_sdk`), un `manifest.ron`, le code dans `src/lib.rs` et les textes dans `locales/en` et `locales/fr`. Il ajoute une fenêtre, deux actions au menu **Outils** et une page de réglages.
2. **Construire l'éditeur.** Depuis la racine du dépôt : `cargo build --release -p orbit_editor`.
3. **Empaqueter le plugin.** Lancez `cargo xtask plugins --examples --only orbit_hello_plugin`. La commande construit le plugin avec la chaîne d'outils et le profil de l'éditeur, vérifie qu'il résout les crates partagées comme l'éditeur, et écrit `<id>-<version>-<système>.orbitplugin` dans le dossier `plugins` de votre dossier cible cargo. Ajoutez `--debug` si votre éditeur est une version de debug.
4. **L'installer.** Lancez l'éditeur, puis choisissez **Outils > Extensions > Plugins…**. Cliquez sur **Installer depuis un fichier…**, choisissez le paquet et lisez l'avertissement **Installer du code natif ?** : un plugin a les mêmes droits que l'éditeur. Cliquez sur **Installer** puis sur **Redémarrer maintenant**.
5. **L'essayer.** Le menu **Outils** contient maintenant **Fenêtre Hello** et **Ajouter une note**. Avec un projet ouvert, **Ajouter une note** ajoute une réplique de narrateur à la scène courante en une seule étape d'annulation : appuyez sur Ctrl+Z pour la voir disparaître. Dans le gestionnaire de plugins, l'entrée Hello affiche **Activé — en cours d'exécution** et un bouton **Réglages** qui ouvre sa page de réglages (**Texte des notes :**).
6. **La rendre vôtre.** Copiez le dossier `hello` à côté de lui et renommez-le. Changez quatre choses : le `name` du paquet dans `Cargo.toml` ; l'`id` dans `manifest.ron` ; la constante `ID` et le `id = "…"` de l'appel `export_plugin!`, qui doivent correspondre au manifeste (un nom pointé en minuscules comme `acme.salut`) ; et les textes.
7. **Ajouter une action.** Dans `on_load`, enregistrez une autre action de la même façon que les existantes :

```rust
host.action(
    ActionDef::new("salut", "salut.salut", |ctx| ctx.info("Salut depuis mon plugin !"))
        .category("category.tools")
        .menu("tools", 902),
);
```

8. Donnez-lui un libellé dans `locales/fr/salut.ron` avec `"salut.salut": "Saluer le monde",` et l'équivalent anglais dans `locales/en`. L'éditeur charge lui-même les textes du paquet.
9. Reconstruisez avec la même commande `cargo xtask plugins`, installez le nouveau paquet depuis le gestionnaire de plugins (une nouvelle version s'installe par-dessus la précédente) et redémarrez. **Saluer le monde** apparaît dans le menu **Outils**.
10. Pour construire hors du dépôt, mettez dans votre `Cargo.toml` `orbit_editor_sdk = { git = "…/OrbitEngine", tag = "vX.Y.Z" }` avec l'étiquette de l'éditeur, et copiez le `Cargo.lock` et le `rust-toolchain.toml` de l'éditeur à côté.

### Résultat

Un plugin à vous, installé depuis un paquet `.orbitplugin`, qui ajoute une action à l'éditeur et que l'on peut retirer du gestionnaire de plugins avec **Désinstaller**.

### Pour aller plus loin

- L'`EditorContext` des closures modifie le projet comme les outils intégrés : `transaction` et `group` regroupent plusieurs opérations en une seule entrée d'annulation, `call_tool` utilise les outils MCP, et `spawn_task` lance un travail en arrière-plan avec barre de progression et **Annuler**.
- Ajoutez un panneau (`panel`), une fenêtre flottante (`window`), un inspecteur (`inspector`) ou une page de réglages (`settings_page`) depuis `PluginHost`.
- Un plugin doit être reconstruit pour chaque nouvelle version de l'éditeur. Le gestionnaire de plugins affiche **Incompatible** et liste ce qui diffère quand ce n'est pas le cas.

### En cas de problème

- **Incompatible** : construit pour une autre version de l'éditeur. Reconstruisez avec `cargo xtask plugins` depuis les mêmes sources et le même profil que l'éditeur.
- **Désactivé après un plantage** : une panique dans votre code désactive le plugin au lieu de fermer l'éditeur. Corrigez-la et cochez de nouveau **Activé**.
- **L'éditeur s'est arrêté pendant le chargement du plugin** : il est désactivé au démarrage suivant. Vous pouvez aussi lancer l'éditeur avec `--no-plugins` (mode sans échec) pour ne rien charger et utiliser le gestionnaire pour désinstaller.
- **« L'identifiant est déjà utilisé »** : l'`id` est pris par un autre plugin ou par un outil intégré. Choisissez-en un autre.
- **L'action manque dans le menu** : vérifiez le redémarrage (les changements ne prennent effet qu'après un redémarrage) et que la clé du libellé existe dans votre fichier `locales`.
