# Changelog

## Non publié

### Changements
- **Dialogues de fichiers pour les plugins** : `EditorContext::pick_folder`, `pick_file` et `save_file` (types `std` seulement, `rfd` reste dans l'éditeur). Les plugins officiels Ren'Py, Rapport des ressources, Script de doublage, Statistiques et Analyse de l'histoire l'utilisent : plus de `rfd` dans leurs bibliothèques, et le serveur MCP est marqué « occupé » pendant ces dialogues.
- **L'import Ren'Py de l'éditeur devient un plugin** (`orbit.renpy_import`, `plugins/orbit_renpy_plugin`) : l'éditeur n'embarque plus `orbit_renpy`. Compilez-le avec `cargo xtask plugins` et installez-le depuis le gestionnaire de plugins ; Fichier > Importer depuis Ren'Py et le rapport d'import fonctionnent comme avant une fois installé (le bouton de la fenêtre Nouveau projet disparaît). `orbit import-renpy` est inchangé.
- **Serveur MCP de l'éditeur en plugin** : le serveur MCP live (HTTP sur 127.0.0.1, jeton) et le panneau Assistant IA deviennent le plugin officiel **Serveur MCP** (`orbit.mcp`, `plugins/orbit_mcp_plugin`, compilé par `cargo xtask plugins`). L'éditeur ne compile plus tokio, hyper ni rmcp (arbre de dépendances de `orbit_editor` : 419 → 374 lignes) ; il garde les outils MCP et les exécute lui-même, si bien que les plugins qui appellent des outils (Studio IA) fonctionnent sans le plugin. Le port et le jeton enregistrés sont repris. Sans le plugin, le robot de la barre d'état est masqué et `--mcp-port` signale le plugin manquant. `orbit mcp` (ligne de commande) est inchangé.
- Pour les développeurs : `orbit_mcp` a une feature `server` (transports stdio et HTTP, désactivée par défaut, activée par `orbit_cli` et le plugin) ; l'API de l'éditeur gagne `orbit_editor_api::mcp` (file de requêtes `McpQueue`, trait `McpTransport`, `PluginHost::mcp_transport`).

### Nouveautés

**Plugins de jeu dans le Store**
- **Plugins de jeu (projet)** : l'onglet Store du gestionnaire de plugins a un sélecteur « Plugins d'éditeur / Plugins de jeu (projet) ». Les plugins de jeu (étapes et actions personnalisées exécutées dans le jeu) sont distribués en sources, avec un badge **Jeu**, la version proposée pour ce moteur (version d'API de plugins et plage de versions du moteur) et leur état dans le projet ouvert.
- **Installation par projet** : un plugin de jeu s'installe dans `plugins/<nom>/` du projet (jamais dans le dossier des plugins de l'éditeur) et s'ajoute à la liste des plugins requis de `orbit.project.ron` ; mise à jour (depuis sa source d'installation uniquement) et désinstallation depuis l'onglet Installés, section « Plugins de jeu de <projet> ».
- **Sécurité** : mêmes niveaux de confiance que les plugins d'éditeur (Officiel, Tiers signé, Non signé), signature Ed25519 propre aux paquets de sources, taille et SHA-256 vérifiés, chaque fichier du paquet haché ; chemins d'archive hostiles (`..`, chemins absolus, lecteurs, liens, noms réservés Windows), doublons et bombes zip refusés avant toute écriture ; avertissement « code compilé et exécuté à l'export » avant toute installation tierce.
- **Modifications locales protégées** : un dossier modifié depuis son installation, une crate écrite à la main au même endroit ou fournissant le même identifiant ne sont jamais écrasés en silence ; « Remplacer (garder une sauvegarde) » les déplace dans `.orbit/plugin-backups/`.
- **Partie éditeur et toolchain** : l'onglet Installés dit si l'éditeur connaît les types de nœuds du plugin (sinon ses étapes restent en lecture seule) et si l'export pourra le compiler (toolchain Rust, `ORBIT_ENGINE_SRC`) ; sans eux, le plugin s'installe mais l'export échouera, et l'éditeur le dit.
- **Auteurs** : `cargo xtask game-plugin pack <crate> --sign <clé>` produit un paquet `.orbitsrc` signé, `cargo xtask game-plugin index` l'ajoute au champ `game_plugins` d'un catalogue. Le plugin d'exemple `examples/plugins/dice` est empaqueté et installé de bout en bout dans les tests.

### Compatibilité
- `index.json` reste au schéma 1 : `game_plugins` est un champ facultatif, ignoré par les éditeurs précédents ; un catalogue sans ce champ est inchangé, et `cargo xtask plugin-index` le conserve.

## 0.4.0 (2026-11-05)

Version centrée sur l'import de jeux Ren'Py, le point & click, les notifications en jeu, les personnages 3D animés et un éditeur extensible par plugins.

### Nouveautés

**Import Ren'Py**
- **Importer un jeu Ren'Py** (Fichier > Importer, exporter et traductions > Importer depuis Ren'Py, ou `orbit import-renpy`) : le dossier d'un jeu Ren'Py devient un nouveau projet qui valide. Labels, sauts et appels, menus avec conditions, variables (y compris persistantes), personnages et couleurs, images, décors et positions, transitions, musique, sons, voix, balises de texte et traductions sont convertis ; le reste est conservé en script commenté et listé dans un rapport d'import affiché après l'opération.
- **Transitions, positions et ATL** : les `at`, `Transform()`, `Position()`, transformations nommées et animations ATL simples deviennent des positions, des échelles et des timelines ; fondus de couleur, `time_warp` et `Pause()` sont pris en charge, les transitions approximées sont signalées une seule fois.
- **Images de script** : `Solid`, `im.*`, `Composite`, `ConditionSwitch`, `layeredimage` et images animées par frames sont convertis au mieux.
- **Styles de personnages** : couleur, préfixe, suffixe et italique des répliques sont appliqués aux lignes et aux traductions ; `extend` est fusionné après `{nw}`.
- **Écrans de navigation** : `imagemap` et `imagebutton` deviennent des cartes point & click (les boutons image deviennent des zones image dessinées), `textbutton` des choix ; les actions simples (variables, notification, saut, retour) deviennent des actions de zone.
- **Succès, persistance, NVL et notifications** : déclarations de succès et de variables persistantes, `renpy.music` / `sound` / `pause` / `jump` / `call`, et `renpy.notify` / `Notify()` qui deviennent des nœuds Notification.

**Point & click**
- **Zones cliquables** : nouveau nœud Zones cliquables pour la navigation point & click. Chaque zone (rectangle, ellipse, polygone) sur le décor a une infobulle, une cible, des conditions de visibilité et d'activation, une image et un son au survol. Dans le jeu : léger scintillement au repos, halo pulsé au survol, curseur main, navigation au clavier, à la manette et par chiffres, touche V pour afficher toutes les zones.
- **Dessiner les zones dans l'Éditeur de vue** : rectangle et ellipse par glisser, polygone par clics ; déplacement, redimensionnement, ajout et suppression de sommets, une étape d'annulation par geste. Inspecteur avec une carte par zone.
- **Actions de zone** : une zone peut, au clic, modifier des variables (sans code), afficher une notification, jouer un son, puis rester sur la carte (ramasser un objet), aller à un label ou une scène, appeler un label (retour sur la carte) ou revenir d'un appel. Inspecteur, texte (`do …` sous un `spot`), MCP et validation.
- **Zones image** : une zone peut être une image dont seuls les pixels opaques réagissent (un objet posé sur le décor), avec un halo qui suit son contour ; l'image peut être dessinée par-dessus le décor. Les `imagebutton` importés de Ren'Py deviennent de telles zones : leurs images sont maintenant affichées.
- **Manette** : nouvelle action « afficher les zones » (Vue / Select par défaut, assignable à Y), avec un rappel sous l'infobulle quand la manette est utilisée. La touche V reste disponible.
- **Assistant « Nouvelle carte de navigation (maison) »** (Édition > Insérer, palette) : pièces, décors, portes entre les pièces et exemple de porte fermée à clé, en une seule étape d'annulation.

**Notifications**
- **Nœud Notification** : affiche un message en jeu (balises, `[variable]`), avec icône, son, durée et style nommé, sans interrompre l'histoire. Disponible dans la palette, l'inspecteur, le texte (`notify "…"`), les scripts Rhai (`notify(...)`), les plugins et les actions de bouton.
- **Onglet Notifications de l'éditeur d'interface** : position, largeur, nombre visible, empilement, durée, animation, son, fermeture au clic, comportement pendant le passage rapide, et styles nommés (défaut, succès, système, personnalisés) avec un bouton « Afficher une notification de test ».
- Les succès débloqués, les messages du moteur et les notifications de l'histoire passent par une seule pile : sons à l'apparition, fondu en mouvement réduit, lecture par le lecteur d'écran. Un chargement, un retour en arrière ou une nouvelle partie effacent les notifications de l'histoire.

**Dialogue**
- **Bulles de bande dessinée** : nouveau mode d'affichage des dialogues, en bulles placées près du visage du personnage, avec queue vers la tête, formes parole, pensée (nuage) et cri (éclat) choisies par réplique avec `{bubble=…}`, boîte du narrateur, réglages par personnage. Onglet Dialogue de l'éditeur d'interface et aperçu.

**Interface du jeu**
- **Musique des menus** : musique par écran, remplacement ou atténuation de la musique du titre et du jeu, actions Jouer / Arrêter la musique.
- **Emplacements de sauvegarde personnalisables** : modèle de carte, champs liés aux données de la sauvegarde et styles selon l'état.
- **Listes longues** : la galerie, les succès et le rejeu défilent par ligne (molette, glisser, clavier, manette, barre) ou se divisent en pages avec Précédent / Suivant lorsqu'elles ne tiennent plus à l'écran.

**3D**
- **Personnages glTF animés** : les modèles importés gardent leur squelette, leurs poids et leurs animations. Mode pose dans l'éditeur 3D (sélection des os, rotation et déplacement avec le gizmo, annulable), arbre des os, poses nommées, aperçu des clips glTF avec scrub, cuisson en pose et clips image par image.
- **Outils de la fenêtre 3D** : vues prédéfinies (1 / 3 / 7, Ctrl pour l'opposé), magnétisme, frustums des caméras, gizmos des lumières, aperçu de la caméra du jeu en incrustation, aligner la caméra sur la vue, poser au sol (G), glisser-déposer de modèles depuis la hiérarchie ou de fichiers glTF.
- **Scènes 3D dans les histoires** : l'aperçu d'une scène 2D qui affiche une scène 3D dessine maintenant ses modèles, pas seulement son décor. L'éditeur 3D signale quand l'histoire utilise une copie périmée de la scène ouverte et propose de reprendre le fichier de l'histoire ; nouveau bouton « Afficher dans l'histoire ».

**Éditeur**
- **Menus imbriqués** : le menu Affichage se divise en Panneaux, Disposition et Options ; les menus Fichier, Édition et Outils gagnent des sous-menus.
- **Modifier dans l'inspecteur** : clic droit ou double clic « Modifier » sur une ligne de scène, un bloc du graphe, une entrée de l'explorateur, un problème ou la scène ouvre directement l'inspecteur.
- Le menu Insérer un nœud est organisé en sous-menus par catégorie (Scène, Effets, Écrans, 3D…), avec l'entrée « Nouvelle carte de navigation (maison) ».
- Export : bouton Parcourir pour choisir l'icône du jeu.

**Plugins de l'éditeur**
- **Plugins de l'éditeur (DLL)** : l'éditeur charge au démarrage des plugins natifs installés par l'utilisateur depuis **Outils > Extensions > Plugins…** (installation d'un `.orbitplugin` ou d'une bibliothèque, activation, désactivation, désinstallation, réglages, redémarrage). Chaque plugin est vérifié avant chargement : format du fichier, description lue dans un processus séparé, empreinte de compilation identique à celle de l'éditeur (version, compilateur, egui, sources, disposition et identité des types), somme de contrôle. Un plugin qui plante est désactivé sans fermer l'éditeur ; `--no-plugins` démarre sans aucun plugin.
- SDK des plugins (`orbit_editor_sdk`), exemple `examples/editor_plugins/hello`, chapitre « Plugins de l'éditeur » du manuel, et `cargo xtask plugins` pour compiler et empaqueter les plugins officiels.
- **Studio IA** (livré en plugin, voir Changements) : bible de l'histoire, écriture du script et génération d'images via Ollama, ComfyUI ou Automatic1111 ; modèles Stable Diffusion (préréglages SD 1.5 / SDXL / Pony / Illustrious, LoRA, VAE, clip skip), listes de modèles lues sur le disque, recherche et téléchargement vérifié depuis Civitai, LoRA et graine par personnage, réparation automatique des références manquantes de la bible. Chapitre du manuel.

**Outils MCP**
- Zones cliquables (ajout, édition, actions, zones image), nœud Notification et réglages des notifications, poses des personnages 3D, outils du Studio IA.

### Corrections
- **Player web** : le jeu exporté pour le web restait bloqué sur l'écran de démarrage au premier chargement de scène.
- **Icône du jeu exporté** : l'icône choisie à l'export est maintenant écrite dans l'exécutable Windows.
- L'Éditeur de vue montre le décor propre d'une carte point & click même sans nœud Décor avant elle.
- Une scène 3D importée dans la bibliothèque garde son nom et son dossier `models/` ; « Ajouter au projet » copie les modèles avec la scène, et les copies plus anciennes sont remplacées. Le panneau Problèmes signale une scène 3D qui place un modèle absent de son dossier.
- Annuler une modification du texte de l'histoire dont les nouveaux nœuds se référencent entre eux (un menu avant ses labels) n'échoue plus.
- La ligne de commande affiche les notifications pendant la lecture d'un jeu.

### Changements
- **Le Studio IA devient un plugin** : il n'est plus intégré à l'éditeur livré. Compilez-le avec `cargo xtask plugins` et installez-le depuis le gestionnaire de plugins ; ses réglages existants (`ai_studio.ron`) sont conservés.
- L'import Ren'Py n'est jamais inclus dans le player ni dans le jeu exporté.

### Compatibilité
- Les anciens projets et sauvegardes se chargent toujours ; les fichiers d'interface passent en `ui_version` 11 (migrations automatiques), les thèmes sans réglage de notifications ni de bulles sont réécrits à l'identique.
- Les jeux qui utilisent les notifications sont empaquetés en pack version 8, ceux qui utilisent les actions de zone ou les zones image en version 9 ; les autres gardent leur version, à l'octet près. Les players plus anciens refusent ces packs comme trop récents.

## 0.3.1 (2026-10-07)

Version corrective.

### Corrections
- **Clignements des yeux dans le jeu exporté** : une image utilisée à la fois par un rig et déclarée comme image du projet (par exemple des paupières fermées) pouvait être libérée au chargement, et le rig ne l'affichait plus. Les clignements fonctionnent désormais dans le jeu exporté comme dans l'aperçu de l'éditeur. Réexportez votre jeu pour en profiter.

### Export
- Nouvel avertissement quand une clé de clip affiche une image absente de tous les costumes du rig.
- Nouvel avertissement quand une image de rig est rangée hors du dossier du rig (elle reste incluse dans le jeu).

## 0.3.0 (2026-10-07)

Version centrée sur l'habillage de l'interface, les succès, le doublage et un éditeur de rig beaucoup plus complet, avec un éditeur nettement plus fluide.

### Nouveautés

**Interface du jeu**
- Boutons image, éditeur visuel de styles et boîte de dialogue entièrement stylable.
- Espaces de travail d'outils sous forme de groupes d'onglets dans le dock, avec zones d'accueil et disposition versionnée.

**Succès et doublage**
- Succès : nœud Déblocage, déblocages persistants, notification, écran des succès, point d'accroche pour les plateformes, icône verrouillée configurable.
- Doublage : voix par réplique et par langue, mode auto qui attend la fin de la voix, bouton de réécoute, import par lot.
- Panneau Succès dans l'éditeur, manuel mis à jour (français et anglais), démo enrichie.

**Éditeur de rig**
- Parcours « de l'image PNG aux os » plus guidé.
- Import de calques depuis des fichiers PSD, Krita et OpenRaster (ORA).
- Outils de découpe et de peinture : découper une image en calques, peindre, effacer, cloner, remplir, pipette et sélections de découpe, y compris en mode pose sur le rig animé. Édition dans une application externe, ordre d'affichage par glisser-déposer.
- Dupliquer un rig, copier des clips depuis un autre rig, recentrer sur l'origine, indice pour les slots masqués.
- Images partagées en copie sur écriture : modifier une image ne touche plus les autres parties qui l'utilisent.

**Outils MCP**
- Construire et animer un rig 2D de bout en bout (création, os, liaison, slots, skins, IK, clips, clés), importer des calques, découper des images.
- Gestion des succès et de leurs icônes.

**Bibliothèque**
- Sélection multiple.

### Performances

- Chargement d'un rig : de près de 90 s à environ 50 ms (sonde de version corrigée).
- Tâches de fond avec progression, annulation et détection de résultats périmés : ouverture, sauvegarde, duplication de rig et chargeurs ne bloquent plus l'éditeur.
- Fin des gels : détecteur de saccades, écriture des images peintes en arrière-plan, journal d'erreurs asynchrone, préchauffage 3D.
- Panneaux à cache paresseux selon leur visibilité, chargement des aperçus à la demande, corrections de plusieurs lenteurs quadratiques.
- Validation hors de la boucle d'affichage : bouton Valider (Ctrl+F7) et option de validation automatique.

### Corrections

- Les fichiers d'interface et les icônes de succès sont désormais exportés et chargés avec le jeu.
- Le MCP en direct ne se bloque plus sur `tools/list` ; messages « éditeur occupé » immédiats.
- Meilleure gestion de la perte du périphérique GPU.
- Les touches d'outils sont ignorées lorsqu'un champ texte a le focus ; panneau d'export sans profils superflus.
- Fichiers de référence de l'interface corrigés.

### Compatibilité

- Formats : schéma d'histoire v16, pack jusqu'à v7 (uniquement lorsque les nouvelles fonctions sont utilisées), `ui_version` 8. Les anciens projets et sauvegardes se chargent toujours.
- Le player natif démarre directement sur les écrans de démarrage (le clic pour démarrer reste sur le web).

### Limites connues

- La 3D reste expérimentale et masquée par défaut.
- L'export Android n'a toujours pas été testé sur appareil.
- Les installeurs ne sont pas signés (SmartScreen sous Windows).
- Le player web 2D dépasse encore le budget de taille.

## 0.2.0 (2026-10-03)

Grosse version fonctionnelle : l'éditeur couvre maintenant l'animation, l'interface, la localisation, l'accessibilité et le scripting, en plus de l'écriture de récits sans code.

### Nouveautés

**Écriture et projet**
- Nouvelle extension de fichier `.orbit` pour les histoires (les anciens `.ron` restent lisibles, conversion depuis l'éditeur), avec association de fichiers dans l'installeur.
- Variables sans code (y compris listes et tables), expressions animées.
- Vue texte de l'histoire (DSL) synchronisée avec le graphe, vérifications dans le panneau Problèmes.
- Modèles de projet (vide, démarrage visual novel) et manuel intégré (Aide > Manuel, français et anglais).
- Graphe : nœud Script avec branches et sortie d'erreur, connexion et coupe de liens.

**Animation**
- Courbes nommées et easing de transitions, éditeur de courbes.
- Tweens, pistes, clips, nœuds Effet et Timeline avec éditeur de timeline et aperçu par scrub.
- Calques de fond en parallaxe et particules 2D.
- Animation squelettique 2D : os, slots, skins, IK, maillages déformables à partir d'images, peinture de poids, clips de rig, expressions animées par rig.
- Éditeur de rig pour débutants : nouveau rig depuis une image PNG, parties d'image, liaison aux os, barre d'étapes, glisser-déposer.

**Interface du jeu**
- Écrans, styles, HUD, inventaire, galerie (déblocages, rejeu de scènes), transitions, menus, écrans de démarrage, polices nommées, grilles, 9-slice, listes défilantes, vrais menus déroulants.
- Éditeur d'interface avec Ctrl+S / Ctrl+Z, rechargement à chaud pendant le test.
- Scènes animées, images GIF/APNG, séquences à taille variable.

**Localisation et accessibilité**
- Échange PO/CSV, détection des traductions obsolètes, panneau Traductions.
- Chaînes de polices par langue, césure japonaise/chinoise, alignement droite-gauche, avertissements de glyphes manquants.
- Lecteur d'écran (bureau et web), manette, touches configurables, taille du texte, contraste élevé, mouvement réduit.

**Scripting et extensions**
- Scripts Rhai (nœud Script, conditions) avec bac à sable renforcé après revue de sécurité.
- Plugins Rust à l'exécution (nœuds et actions personnalisés), export avec plugins, exemple « dice ».
- Outils MCP étendus (interface, traductions, rig, 3D, scripts, DSL).

**Export et performances**
- Player web plus léger, template web avec script, profil de qualité 3D à l'export.
- Export Android (APK/AAB) depuis un player précompilé.
- Profileur intégré (panneau éditeur, superposition F3 dans le player, `cargo xtask bench`).
- Reprise après perte du GPU dans l'éditeur au lieu d'un plantage.

### Expérimental

- **3D** : scènes 3D, éditeur de scène, formes procédurales, import glTF, ombres, ciel, post-traitement, template web 3D. Ces fonctions sont **expérimentales et masquées par défaut** ; activez-les dans le menu Affichage > Fonctionnalités 3D expérimentales. Format et comportement susceptibles de changer.

### Limites connues

- **L'export Android n'a pas été testé** sur appareil.
- Le player web 2D dépasse le budget de taille fixé en CI (décision en attente).
- Pas de signature de code ni de mise à jour automatique : Windows affiche SmartScreen à l'installation.
- Le schéma des projets a évolué ; les anciens projets sont migrés à l'ouverture, faites une sauvegarde avant.

## 0.1.0

Première version interne (pré-alpha).
