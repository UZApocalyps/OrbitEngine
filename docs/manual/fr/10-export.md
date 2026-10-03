# Exporter votre jeu

Un export produit un jeu que les joueurs lancent **sans rien installer** : un `.zip` pour Windows, un `.tar.gz` pour Linux, un dossier à mettre sur un site web, ou un `.apk` / `.aab` pour Android.

## Depuis l'éditeur

Choisissez **Fichier > Exporter le jeu** pour ouvrir le panneau Export.

1. **Profil d'export** : le nom du jeu, un **identifiant** (le dossier où sont rangées les sauvegardes des joueurs : minuscules, chiffres, `-` et `_` ; ne le changez plus après publication), la version, une icône (un PNG carré, 256x256 conseillé), la langue par défaut et la compression. Enregistrez les profils pour les garder avec le projet ; vous pouvez en avoir plusieurs (par exemple `demo` et `release`).
2. **Cibles** : Windows, Linux, Web et Android (voir plus bas). Une cible dont le player est introuvable est signalée.
3. **Dossier de sortie** : `export/` dans le projet par défaut.
4. **Exporter** : une barre de progression et un journal montrent chaque étape et les avertissements éventuels (assets inutilisés, traductions incomplètes). **Annuler** l'arrête. L'histoire est enregistrée d'abord, et les erreurs du panneau Problèmes bloquent l'export.
5. À la fin, ouvrez le dossier, et pour le web utilisez **Tester dans le navigateur**.

## Ce que reçoit le joueur

- **Windows** : un seul `.exe` contenant le jeu, et un court README.
- **Linux** : un seul exécutable ; un pilote graphique (Vulkan ou OpenGL) est nécessaire.
- **Web** : un dossier à mettre tel quel sur n'importe quel hébergeur statique (itch.io, GitHub Pages, Netlify...). Le jeu se charge scène par scène pour démarrer vite. Le plein écran est dans les préférences ou sur F11. La partie est sauvegardée quand l'onglet est masqué.

Les sauvegardes sont rangées sous l'identifiant du jeu dans le dossier de données de l'utilisateur (le stockage du navigateur pour la version web).

## Android

Android est une cible d'export comme les autres : cochez **Android** dans les Cibles du panneau Export et une section **Android** apparaît avec les options ci-dessous. Le player existe en version Android, avec commandes tactiles, et la partie est sauvegardée automatiquement à la mise en pause de l'application.

- **Format** : un **APK** (un fichier installable, pour tester sur un appareil ou distribuer soi-même) ou un **AAB** (le paquet que demande le Play Store ; il nécessite *bundletool*).
- **Identifiant d'application** : l'identifiant unique de l'application (`com.studio.jeu`). Vide, il est dérivé de l'identifiant du jeu. Ne le changez plus après publication.
- **Code de version** : un entier qui doit augmenter à chaque publication sur le store. Vide, il est dérivé de la version (1.2.3 donne 10203).
- **Orientation** : paysage, portrait ou automatique.
- **Signature** : un paquet Android doit être signé. Choisissez soit **votre keystore** (le fichier et l'alias de la clé sont mémorisés dans les préférences de l'éditeur, jamais dans le projet ; le **mot de passe est demandé à chaque export** et jamais enregistré), soit le **keystore de debug** (le standard Android, créé s'il manque) pour essayer le jeu sur un appareil. Le Play Store refuse le keystore de debug, et il faut garder précieusement votre propre keystore : il sert à chaque mise à jour du jeu.
- Le résultat est `<identifiant>-<version>-android.apk` (ou `.aab`) dans le dossier de sortie. Le jeu demande Android 8.0 (API 26) ou plus récent.

**Outils à installer.** Rien n'est téléchargé pour vous. L'éditeur les cherche aux emplacements habituels et affiche ce qu'il a trouvé dans le cadre **Outils trouvés** (utilisez **Chercher à nouveau les outils** après une installation) ; s'il en manque un, l'export dit quoi installer :

- le **SDK Android**, avec ses *build-tools* (`aapt2`, `zipalign`, `apksigner`) et une *plateforme* Android : installez Android Studio, ou les outils en ligne de commande. Définissez `ANDROID_HOME` si le SDK n'est pas détecté ;
- un **JDK 17 ou plus récent** (`keytool`, `jarsigner`) : celui fourni avec Android Studio convient. Définissez `JAVA_HOME` s'il n'est pas détecté ;
- **bundletool**, pour un AAB seulement : définissez `BUNDLETOOL_JAR` vers son `.jar`, ou placez-le dans le SDK ou dans le `PATH`.

Le **NDK** n'est *pas* nécessaire pour exporter un jeu : il ne sert qu'à construire le player Android lui-même (`cargo xtask templates`). L'export utilise le player Android précompilé, livré avec l'éditeur dans son dossier `templates/android`, comme ceux de Windows, Linux et Web.

## Ligne de commande

L'outil `orbit` fait la même chose depuis un terminal : `orbit export windows --project mon_jeu --out dist`, ainsi que `linux`, `web`, `android` et `all` (les cibles du profil). Pour Android, signez avec `--android-keystore <fichier> --android-alias <nom>` (le mot de passe est lu dans la variable d'environnement `ORBIT_ANDROID_KEYSTORE_PASSWORD`, jamais stocké) ou avec `--android-debug` pour les tests ; le format (APK ou AAB) vient du profil.
