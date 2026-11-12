# L'assistant MCP

OrbitEngine peut travailler avec un assistant IA qui modifie votre histoire à vos côtés. Il utilise **MCP** (Model Context Protocol), un standard qui permet à une application d'IA d'utiliser des outils. L'assistant lit et modifie votre histoire avec les mêmes opérations sûres que l'éditeur lui-même.

## Dans l'éditeur

Le serveur MCP de l'éditeur et le panneau **Assistant IA** sont fournis par le plugin officiel **Serveur MCP** (`orbit.mcp`) : l'éditeur de base ne contient ni serveur réseau ni moteur asynchrone, ce qui le garde plus léger. Les distributions officielles le livrent avec l'éditeur et l'installent au premier lancement ; dans une version compilée à la main, installez-le depuis **Outils > Extensions > Plugins…** (voir le chapitre « Plugins de l'éditeur »). Sans lui, le robot de la barre d'état n'apparaît pas et `--mcp-port` indique que le plugin manque ; le serveur en ligne de commande (plus bas) fonctionne toujours. Le port et le jeton déjà enregistrés sont repris tels quels.

Ouvrez le panneau **Assistant** (menu Outils, Ctrl+Maj+A). Il indique si la connexion est prête et donne l'adresse à renseigner dans votre application d'IA. Tant que l'éditeur est ouvert, l'assistant voit votre projet en direct : ses modifications apparaissent immédiatement et s'annulent avec Ctrl+Z comme les vôtres.

## Ce que l'assistant peut faire

Il ne peut rien faire que l'éditeur refuse : une modification qui casserait l'histoire est rejetée avec une explication, et rien n'est appliqué à moitié. Les outils sont groupés par sujet :

- **Histoire** : lister, lire, créer, renommer et supprimer des scènes ; ajouter et modifier des répliques et des choix ; placer n'importe quelle étape avec `add_command` (décors, personnages, musiques, sons, transitions, attentes, changements de variables, conditions, étiquettes, sauts, appels, **notifications**, scripts, écrans...) ; déplacer et supprimer des étapes ; lire la liste des problèmes ; `validate` ; `simulate` (jouer l'histoire avec une liste de choix, sans carte graphique, pour vérifier chaque branche) ; enregistrer, annuler et rétablir.
- **Définitions** : personnages et expressions, variables, assets et courbes.
- **Point & click** : `add_hotspots` et `edit_hotspots` créent et modifient les zones cliquables (rectangle, ellipse, polygone, zones image), leurs conditions et leurs actions au clic.
- **Interface** : lire et écrire les écrans, les styles, la boîte de dialogue (mode bulles de BD compris), les réglages et styles de **notifications**, les splashes, les emplacements de sauvegarde et la musique des écrans.
- **Succès** : lister, créer, modifier et supprimer les succès et leurs icônes.
- **Voix et traductions** : importer des fichiers de voix en lot (`import_voices`, avec un essai à blanc), et lire la couverture et les traductions périmées de chaque langue (`translation_status`).
- **Scripts** : lister, lire, écrire et vérifier des scripts et modules Rhai, et tester un script dans le bac à sable (voir *Scripts*).
- **Rigs (animation squelettique 2D)** : construire et animer un rig de bout en bout (création depuis une image, os, liaison, slots, skins, IK, clips et clés), importer des calques PSD / Krita / OpenRaster, découper une image en calques, dupliquer et recentrer des rigs, et échantillonner une pose pour la vérifier (voir *Personnages et animations*).
- **Scènes 3D** (expérimental) : lister, lire, créer et modifier des scènes, et poser le squelette d'un personnage 3D. Ces outils n'apparaissent que si l'interrupteur 3D expérimental est activé.
- **Éditeur seulement** (mode live) : jouer, avancer et choisir dans la preview, en prendre une capture d'écran, sélectionner une étape dans l'éditeur, prendre une image d'une scène 3D depuis l'une de ses caméras, et `build_project` / `export_game` (marqués destructifs).
- **Plugins** : les outils déclarés par les plugins de jeu et les plugins de l'éditeur installés (par exemple ceux du Studio IA).

## Bon à savoir

- Chaque requête de l'assistant est une entrée de votre historique d'annulation, étiquetée avec le nom de l'outil.
- Si vous modifiez la même étape en même temps, l'assistant est prévenu que l'étape a changé et réessaie au lieu d'écraser votre travail.
- Supprimer une scène ou une étape est marqué comme destructif, pour que votre application d'IA puisse vous demander avant de le faire.
- Certains outils (rigs, scènes 3D) réécrivent un fichier hors de l'historique d'annulation quand ce fichier n'est pas ouvert dans son éditeur ; quand il l'est, la modification passe par l'éditeur et Ctrl+Z l'annule pour les rigs.

## Sans l'éditeur

Un serveur en ligne de commande (`orbit mcp --project mon_jeu`) offre les mêmes outils quand l'éditeur est fermé. La documentation technique des outils se trouve dans `docs/mcp.md` du dépôt OrbitEngine.
