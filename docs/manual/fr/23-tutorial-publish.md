# Tutoriel : publier votre jeu

Votre jeu tourne dans l'éditeur. Ce tutoriel le transforme en quelque chose que d'autres personnes peuvent jouer : un téléchargement Windows ou Linux, une page web que vous pouvez mettre sur itch.io ou sur n'importe quel site, ou une application Android. Il montre aussi comment tester chaque résultat avant de le partager.

## Objectif

Exporter le jeu pour plusieurs plateformes à partir d'un seul profil, tester la version web sur votre ordinateur, puis la mettre en ligne sur itch.io.

## Avant de commencer

- Un jeu terminé sans erreur rouge dans le panneau **Problèmes** (les erreurs bloquent l'export, les avertissements non).
- Une icône PNG carrée, 256x256 conseillé, importée dans la bibliothèque du projet (par exemple `assets/icon.png`).
- Pour Android seulement : les outils listés dans le chapitre *Exporter votre jeu* (Android SDK, un JDK 17 ou plus récent et, pour le Play Store, bundletool). Les exports Windows, Linux et web n'ont besoin de rien de plus que l'éditeur.

## Étapes

### 1. Ouvrir le panneau Export

1. Choisissez **Fichier > Import, export et traductions > Exporter le jeu…**. Le panneau **Export** s'ouvre.
2. L'histoire est enregistrée d'abord si elle a des modifications non enregistrées. Un message vous le rappelle.

### 2. Remplir le profil d'export

1. Sous **Profil d'export**, écrivez le **Nom du jeu**. Il devient le titre de la fenêtre et les noms de fichiers.
2. Choisissez l'**Identifiant** : minuscules, chiffres, `-` et `_`, par exemple `mon-premier-jeu`. Il nomme le dossier où vivent les sauvegardes des joueurs : ne le changez donc jamais après publication, les joueurs perdraient leurs sauvegardes.
3. Réglez la **Version** (par exemple `1.0.0`) et augmentez-la à chaque publication.
4. Pour l'**Icône (PNG)**, cliquez sur **Parcourir…** et choisissez votre icône parmi les PNG du projet.
5. Réglez la **Langue par défaut** : le code de langue (`fr`, `en`...) utilisé par le jeu quand le joueur n'a pas choisi. Laissé vide, il suit la langue du système du joueur.
6. Cliquez sur **Enregistrer les profils** pour garder le profil dans le projet. Vous pouvez créer plusieurs profils avec **Nouveau** (par exemple `demo` et `release`) et sélectionner celui à exporter.

### 3. Choisir les cibles

Sous **Cibles**, cochez ce que vous voulez :

- **Windows (zip)** : un seul `.exe` contenant le jeu, dans un `.zip`.
- **Linux (tar.gz)** : un seul exécutable dans un `.tar.gz`. L'ordinateur du joueur a besoin d'un pilote graphique (Vulkan ou OpenGL).
- **Web (navigateur)** : un dossier à mettre sur un site. Sous **Web**, **Fichiers .br et .gz précompressés** ajoute des copies compressées pour les hébergeurs qui savent les servir.
- **Android (APK / AAB)** : une section Android apparaît, voir l'étape 7.

Une cible suivie de *(template introuvable)* n'a pas de lecteur installé sur cet ordinateur : l'info-bulle explique où le trouver.

### 4. Exporter

1. Vérifiez le **Dossier de sortie** (`export/` dans le projet par défaut).
2. Cliquez sur **Exporter**. Une barre de progression et le **Journal** montrent chaque étape et les avertissements (assets inutilisés, traductions incomplètes). **Annuler** arrête l'export.
3. Quand c'est terminé, le panneau liste une ligne par cible, avec le fichier et sa taille. Cliquez sur **Ouvrir le dossier** pour voir les résultats, nommés d'après l'identifiant et la version (par exemple `mon-premier-jeu-1.0.0-windows.zip`).

### 5. Tester la version web en local

Un jeu web ne s'ouvre pas en double-cliquant sur `index.html` : les navigateurs refusent WebAssembly sur les pages `file://`. Laissez l'éditeur le servir :

1. Après un export web, cliquez sur **Tester dans le navigateur**.
2. L'éditeur démarre un petit serveur local sur un port libre, affiche **Serveur local :** avec son adresse, et ouvre votre navigateur.
3. Jouez, redimensionnez la fenêtre, appuyez sur F11 pour le plein écran. Fermez l'onglet, puis cliquez sur **Arrêter le serveur**.

Depuis un terminal, le même test est `orbit export web --project <dossier du projet> --out <dossier> --serve`.

### 6. Publier sur itch.io

1. Dans le dossier de sortie, ouvrez le dossier web (`<nom>-<version>-web`). `index.html` doit se trouver directement dedans.
2. Sélectionnez tout ce qui est **dans** ce dossier (pas le dossier lui-même) et compressez-le en un `.zip`. Si vous avez coché **Fichiers .br et .gz précompressés**, vous pouvez supprimer les fichiers `.br` et `.gz`, ou exporter de nouveau avec la case décochée : itch.io ne s'en sert pas, et l'archive est plus petite.
3. Sur itch.io, créez un nouveau projet. Réglez **Kind of project** sur **HTML**, envoyez le `.zip` et cochez **This file will be played in the browser** (l'interface d'itch.io est en anglais).
4. Réglez la taille de la zone de jeu (1280 par 720 convient à la plupart des jeux), activez le bouton plein écran, puis enregistrez et regardez la page. Le jeu se charge scène par scène, il démarre donc vite.
5. Pour tout autre hébergeur statique (GitHub Pages, Netlify, votre propre serveur), envoyez le dossier web entier tel quel. Le fichier `HOSTING.txt` à côté de `index.html` liste ce que le serveur doit faire.

Pour une version Windows ou Linux, envoyez le `.zip` ou le `.tar.gz` sur itch.io comme fichier téléchargeable, marqué pour sa plateforme.

### 7. Publier sur Android

1. Cochez **Android (APK / AAB)**. Dans la section **Android**, choisissez le **Format** : l'**APK** est un fichier installable à tester ou distribuer vous-même ; l'**AAB** est le bundle exigé par le Play Store.
2. Laissez l'**Identifiant d'application** vide pour le dériver de l'identifiant, ou fixez le vôtre (`com.studio.jeu`). Ne le changez jamais après publication. Le **Code de version** doit augmenter à chaque publication sur le store ; vide, il est dérivé de la version.
3. Choisissez l'**Orientation** : **Paysage**, **Portrait** ou **Automatique**.
4. Sous **Signature**, choisissez **Keystore de debug (tests)** pour essayer le jeu sur votre propre appareil, ou **Mon keystore** avec le fichier keystore, l'alias de la clé et le mot de passe, demandé à chaque export et jamais enregistré. Le Play Store refuse le keystore de debug : gardez votre keystore en lieu sûr, il sert à chaque mise à jour.
5. Regardez **Outils trouvés**. Si quelque chose est *introuvable*, installez-le et cliquez sur **Chercher à nouveau les outils**.
6. Exportez. Copiez l'`.apk` sur votre téléphone et ouvrez-le (le téléphone vous demandera d'autoriser l'installation depuis cette source), ou envoyez l'`.aab` à la Play Console.

## Résultat

Un profil, un clic, et votre jeu existe en `.zip` pour Windows, en `.tar.gz` pour Linux, en dossier web testé localement et en ligne sur itch.io, et éventuellement en paquet Android.

## Pour aller plus loin

- Gardez un profil par canal de diffusion (`demo` avec moins de scènes ou une autre icône, `release`), et exportez `all` en ligne de commande : `orbit export all --project mon_jeu --out dist`.
- La qualité 3D des jeux pour ordinateur se règle dans le profil (**Qualité 3D (ordinateur)**) ; les jeux web et Android utilisent toujours le profil web, plus léger.
- Pour une mise à jour, augmentez la **Version**, exportez de nouveau et envoyez les nouveaux fichiers : les joueurs gardent leurs sauvegardes tant que l'identifiant est le même.

## En cas de problème

- **L'export est bloqué ou échoue tout de suite** : ouvrez le panneau **Problèmes** et corrigez les erreurs ; les avertissements ne bloquent pas.
- **« Choisissez au moins une cible »** : cochez une cible.
- **Une cible indique (template introuvable)** : le lecteur de cette plateforme n'est pas installé. Utilisez l'éditeur installé par son installeur, ou indiquez le bon dossier dans **Templates**.
- **La page web reste blanche sur mon site** : elle doit être servie en `http://` ou `https://`, et les fichiers `.wasm` en `application/wasm` (tous les hébergeurs courants le font). Comparez avec **Tester dans le navigateur**.
- **Le jeu démarre lentement chez un hébergeur web** : vérifiez que le serveur répond aux requêtes HTTP Range pour `game.pak` ; sans elles, le lecteur télécharge tout le fichier une fois.
- **L'export Android signale un outil manquant** : lisez la boîte *Outils trouvés* et le message du journal ; définissez `ANDROID_HOME` ou `JAVA_HOME` si un outil est installé mais non détecté.
- **Des joueurs ont perdu leurs sauvegardes après une mise à jour** : l'**Identifiant** (ou l'**Identifiant d'application** Android) a été changé. Remettez l'ancienne valeur.
