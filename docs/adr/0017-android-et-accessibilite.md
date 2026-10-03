# ADR 0017 : Android et accessibilité

Statut : proposé (M7-07 à M7-11, à relire avant M7-09 et M7-10). Les bulles de dialogue (TL-05) sont traitées dans l'ADR 0018.

## Contexte

M7 vise un jeu d'exemple sur 4 plateformes (Windows, Linux, web, Android) et un niveau d'accessibilité raisonnable pour un VN : lecture confortable, jouable à la manette et au clavier seul, utilisable avec un lecteur d'écran pour le texte narratif. Contraintes : moteur maison wgpu + winit (ADR 0001), éditeur no-code (l'export Android doit se faire depuis le panneau Export), compatibilité ascendante (§1.3) : préférences et saves existantes restent valides.

## Partie A : Android

### Options examinées

1. **winit + `android-activity` (NativeActivity / GameActivity), wgpu Vulkan avec repli GLES** (retenu). Même boucle que le desktop ; `orbit_platform` absorbe les différences. GameActivity est préféré (meilleure gestion du clavier logiciel et des insets) ; NativeActivity reste en repli.
2. **Export web enveloppé (WebView / TWA).** Écarté comme cible principale : performances et audio moins fiables, mais documenté comme contournement possible.
3. **Gradle complet généré.** Écarté : impose JDK + Gradle + Android Studio à l'auteur.
4. **`cargo-apk` / `xbuild`.** `cargo-apk` est déprécié ; `xbuild` est prometteur mais instable. Retenu à la place : **`cargo-ndk` pour compiler + assemblage APK/AAB maison** avec `aapt2`, `bundletool`, `zipalign`, `apksigner` de l'Android SDK.

### Décisions

* **Toolchain** : le panneau Export détecte Android SDK (build-tools, platform), NDK et JDK (requis par `apksigner`/`bundletool`) ; manque ⇒ message explicatif avec liens, jamais de téléchargement automatique sans accord. Comme pour le web, l'export utilise un **player Android précompilé** (`liborbit_player.so` pour arm64-v8a, et armeabi-v7a/x86_64 en option) livré avec l'éditeur : l'auteur n'a pas besoin de Rust, seulement de l'outillage de packaging. Les plugins Rust (ADR 0015) exigent en plus `cargo-ndk`.
* **Contenu** : le pak (`GamePack`) est placé dans `assets/` de l'APK et lu via `AAssetManager` (`orbit_platform`), sans extraction. Manifeste généré depuis le projet (nom, id de paquet `com.auteur.jeu` validé, version, orientation paysage par défaut, icônes adaptatives générées depuis l'icône du projet).
* **Formats** : APK (test, distribution directe) et AAB (Play Store). minSdk 26, targetSdk à jour de la politique Play au moment de l'implémentation.
* **Signature** : keystore de l'utilisateur (création guidée via `keytool` si besoin) ; mot de passe demandé à l'export, **jamais enregistré** dans le projet ; une clé de debug générée localement pour les APK de test. Le chemin du keystore est une préférence d'éditeur, pas une donnée de projet (évite de le committer).
* **Cycle de vie** : suspend ⇒ sauvegarde automatique silencieuse (slot dédié) + libération de la surface ; resume ⇒ recréation de la surface et des ressources GPU. Le bouton retour = `Cancel` (fermer l'écran du dessus, sinon menu pause).
* **Entrées** : tactile (tap = avancer, appui long = menu, glisser vers le bas = historique), cibles tactiles ≥ 48 dp (avertissement de validation pour les écrans UI trop petits en profil mobile). Stockage : répertoire interne de l'app pour saves et préférences.
* Rendu : profil de qualité « mobile » proche du profil web (ADR 0014).

## Partie B : accessibilité

### Décisions

* **Préférences** (`orbit_save`, champs `#[serde(default)]` ⇒ anciennes préférences valides) : échelle du texte (0,8–2,0, appliquée par le moteur de mise en page, le layout UI se réadapte), police lisible (dyslexie : police fournie en option), contraste élevé, réduction des animations (tweens, transitions et effets à durée 0, pas de tremblement d'écran), vitesse de texte « instantanée », opacité de la textbox.
* **Contraste élevé** : thème dérivé automatique (couleurs de styles poussées vers un ratio WCAG AA ≥ 4,5:1) qu'un auteur peut remplacer par un thème « contrast » explicite. La validation de l'éditeur avertit des styles de texte sous 4,5:1 sur leur fond déclaré.
* **Manette et clavier** : `gilrs` (desktop, Android), Gamepad API (web). Graphe de focus calculé depuis le layout (navigation spatiale), surchargeable par widget (`focus_next`), focus visible obligatoire dans le thème par défaut ; remappage des actions dans les préférences.
* **Lecteur d'écran** — options examinées :
  1. **AccessKit** (retenu en desktop et Android) : arbre d'accessibilité standard (UIA Windows, AT-SPI Linux, macOS, Android), adaptateur `accesskit_winit`. Le shell construit l'arbre depuis les widgets data-driven (rôle, nom = texte localisé, état) et la textbox (région live annonçant locuteur + réplique).
  2. **TTS direct (`tts` crate)** : écarté comme mécanisme principal (contourne le lecteur de l'utilisateur, réglages dupliqués) ; gardé comme option « auto-lecture » (self-voicing) pour les plateformes sans lecteur actif.
  3. **Web** : AccessKit n'a pas d'adaptateur web stable ; on maintient un DOM caché (éléments ARIA hors écran : `aria-live="polite"` pour les répliques, boutons réels pour les choix et menus, synchronisés avec le focus du canvas).
* Les images (CG, fonds) peuvent recevoir une **description** facultative (texte localisable) annoncée à l'affichage ; ajout additif à l'IR (champ optionnel), pris au prochain bump de schéma de M7.

### Compatibilité

* Préférences : tous les nouveaux champs ont une valeur par défaut égale au comportement actuel ; un fichier de préférences ancien se charge à l'identique.
* Les descriptions d'images et ajouts UI (`focus_next`) sont additifs (schéma / `ui_version` + migration par changement de numéro) ; aucun jeu existant ne change de rendu tant que le joueur n'active pas d'option.
* Les paks antérieurs restent lisibles sur Android (même `GamePack`).

## Conséquences

* M7-09 : AccessKit desktop/Android + DOM ARIA web ; M7-10 : GameActivity, `AAssetManager`, sauvegarde au suspend ; M7-11 : assemblage maison (`aapt2`/`bundletool`/`apksigner`), player précompilé, keystore jamais stocké dans le projet.
* Dépendance externe lourde (SDK/NDK/JDK) assumée et détectée, pas embarquée.

## Addendum : lecteur d'écran (M7-09, implémenté)

* **Modèle** : `orbit_ui::a11y` construit un arbre sans OS (`A11yTree`) depuis les widgets de l'écran visible (`Screen::widgets`, `Ui::access_nodes`) : conteneurs et panneaux aplatis, images décoratives omises, bouton « carte » nommé par ses labels internes, contrôle sans libellé nommé par le label qui le précède dans sa ligne (et valeur d'un slider = label qui le suit). Une modale ouverte masque l'écran dessous (rôle `Dialog`). Le shell ajoute la textbox comme **région live polie** (`« Locuteur : texte »`, balises retirées ; une réplique identique à la précédente reçoit une variante invisible pour être réannoncée), les choix (liste de boutons, focus sur l'option sélectionnée), « Fin », et le dernier toast (seconde région live). Seul l'écran du dessus est exposé. Les identifiants sont stables (emplacement d'écran + index de widget) et les actions reviennent au shell : `Click` = focus + Confirm, `Increment/Decrement` = Droite/Gauche, `Click` sur un choix = choisir, bouton « Cliquer pour démarrer » avant le lancement.
* **Desktop** : `orbit_platform` (option `AppConfig::accessibility`, activée par le player seulement, pas par l'éditeur) crée un adaptateur `accesskit_winit` 0.32 (même `accesskit` 0.24 qu'egui) avant d'afficher la fenêtre. L'app n'est interrogée (`App::accessibility_tree`) qu'**après qu'un lecteur d'écran s'est connecté** : coût nul sinon. Mise à jour complète à chaque changement (comparaison avec l'arbre précédent). Android : l'adaptateur sera branché par M7-10 (feature `accesskit_android`).
* **Web** : DOM caché `#orbit-a11y` (hors écran) : une région `aria-live="polite"` pour répliques et toasts, une seconde pour annoncer le contrôle focalisé (le canvas garde le focus clavier), et de vrais `<button>` (`role="checkbox|slider|combobox"`, `aria-checked`, `aria-valuetext`) dont le clic revient comme `ActionRequest`. Le calcul du miroir (`WebView::from_update`) est testé hors navigateur.
* **Préférence** : `Preferences::screen_reader` (`#[serde(default)]`), **activée par défaut**, exception assumée à « défaut = comportement actuel » : elle ne change ni le rendu ni le son, ne coûte rien sur desktop sans lecteur actif, et un joueur aveugle ne pourrait pas trouver l'option pour l'activer. Les anciens fichiers de préférences se chargent avec la valeur `true`. Interrupteur dans l'écran de préférences standard (`ui.prefs.screen_reader`) ; désactivée, l'arbre se réduit à la fenêtre et les actions sont ignorées.
* **Reporté** : descriptions d'images (champ IR optionnel) — exige un bump de schéma, réservé à la prochaine tâche schéma de M7 (file sérialisée de `migrate.rs`) ; clé `SettingKey::ScreenReader` pour les écrans de préférences data-driven (format UI + éditeur + MCP) ; auto-lecture TTS (« self-voicing »).

## Addendum : taille du texte, contraste élevé, animations réduites (M7-07, implémenté)

* **Préférences** (`orbit_save::Preferences`, `#[serde(default)]`, défaut = comportement actuel) : `text_scale` (1.0, borné à 1.0..=1.6, pas de 0.1 dans les écrans), `high_contrast` (false), `reduced_motion` (false). Les anciens fichiers se chargent inchangés.
* **Taille du texte et contraste** : le thème du projet n'est jamais modifié ; `Theme::accessible(scale, high_contrast)` (`orbit_ui_data::access`) en dérive le thème de dessin. `scaled` multiplie les tailles de texte (textbox, choix, écrans, widgets, styles) et agrandit les boîtes qui portent du texte (la textbox grandit vers le haut en gardant son bord bas, les boutons de choix grandissent) ; les hauteurs de contrôles ne changent pas (elles laissent déjà la place jusqu'à 1,6x), et les écrans standard passent en hauteur « Fit » pour rester dans le canevas. `high_contrast` remplace les couleurs (blanc sur noir, jaune, bordures épaisses, styles nommés conservés avec leurs couleurs remplacées). Le shell garde `base_theme` et `theme`, recalcule à chaque changement et re-skinne les écrans (`Screen::set_theme`) et la ligne ou le choix à l'écran (`Game::set_theme`, affichés en entier).
* **Contrôle WCAG AA** : `Theme::contrast_issues` (ratios WCAG 2.1, 4,5:1 texte, 3:1 composants ; un fond translucide est jugé sur un fond noir et blanc). `UiBundle::validate` les remonte en avertissements (écran `theme`), donc dans le panneau Problèmes de l'éditeur, dans `list_ui` (MCP) ; l'onglet Thème les liste, traduits. Le mode contraste élevé réussit toujours le contrôle.
* **Animations réduites** : `Director::set_reduced_motion` : transitions = coupes, `vm.complete_anims()` après chaque pas (toute animation finie atteint son état final exact, y compris celles qui ne sont pas attendues), les `Effect`/`Timeline` bloquants ne bloquent plus. Les animations infinies (décoration) et les attentes `Wait` de l'auteur sont conservées ; le marqueur « continuer » ne clignote plus. L'activer en cours de partie coupe ce qui joue.
* **Format UI** : `SettingKey` gagne `ScreenReader` (toggle, comble le report de M7-09), `TextScale` (slider), `HighContrast` et `ReducedMotion` (toggles) ; `ui_version` 3 (migration v2 → v3 = numéro seul, fixtures figées `examples/fixtures/compat/ui_v2/`). L'écran de préférences standard (Rust et RON) a une ligne de curseur et deux lignes de deux interrupteurs. Éditeur : liste des liaisons de l'inspecteur ; MCP : rien à ajouter (les écrans passent par `validate`).
* **Reporté** : les écrans de chargement/erreur/démarrage dessinés par le runtime avant le shell ne suivent pas la taille du texte ; les animations infinies ne sont pas arrêtées ; particules (n'existent pas encore) ; mise en page des écrans RON personnalisés à largeurs fixes (l'auteur doit prévoir la place, plage limitée à 1,6x pour cela).

## Mise en œuvre M7-11 (export APK/AAB)

* **Template Android** (`templates/android/`) : `classes.dex` (GameActivity et ses dépendances, compilé avec `d8` depuis l'AAR Games Activity ; `cargo xtask templates --android-dex-input <aar>`), `arm64-v8a/liborbit_player.so` obligatoire, `armeabi-v7a/` et `x86_64/` optionnels, `bundletool.jar` optionnel. `cargo xtask templates` construit les `.so` avec `cargo-ndk` quand cargo-ndk, le NDK et les cibles rust sont présents, sinon saute avec un message.
* **Assemblage** (`orbit_export::android`) : `aapt2 compile/link` produit manifeste et ressources (format proto pour un AAB), puis le zip est écrit par l'export (dex, `lib/<abi>/liborbit_player.so`, `assets/game.pak`). **Le pak et les bibliothèques sont stockés non compressés** (le player lit le pak en place par `AAssetManager`) ; pour un AAB, `BundleConfig.json` met `assets/game.pak` dans `uncompressedGlob`. APK : `zipalign -p`, `apksigner sign`, `apksigner verify` ; AAB : `bundletool build-bundle` puis `jarsigner`.
* **Outils** détectés dans l'ordre `ANDROID_HOME`/`ANDROID_SDK_ROOT` puis dossiers usuels ; `JAVA_HOME`, `java` du `PATH`, JDK d'Android Studio ; `BUNDLETOOL_JAR`. Aucun téléchargement.
* **Signature** : chemin du keystore et alias dans les préférences de l'éditeur (jamais dans le projet) ; le mot de passe est demandé à chaque export, jamais enregistré, transmis par stdin (`apksigner`) ou par l'environnement du processus fils (`jarsigner`), jamais en argument. Option keystore de debug (public, mot de passe `android`) pour les tests ; la clé doit avoir le mot de passe du keystore.
* Identité : id d'application (profil, sinon `com.orbit.<id du jeu>`), `versionCode` (profil, sinon `major*10000+minor*100+patch`), libellé, icône (PNG du profil), orientation. minSdk 26, targetSdk 34.
