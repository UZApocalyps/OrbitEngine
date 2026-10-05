# Changelog

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
