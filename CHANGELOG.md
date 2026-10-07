# Changelog

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
