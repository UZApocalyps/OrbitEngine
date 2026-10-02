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
