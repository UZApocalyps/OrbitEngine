# L'assistant MCP

OrbitEngine peut travailler avec un assistant IA qui modifie votre histoire à vos côtés. Il utilise **MCP** (Model Context Protocol), une manière standard pour une application d'IA d'utiliser des outils. L'assistant lit et modifie votre histoire avec les mêmes opérations sûres que l'éditeur lui-même.

## Dans l'éditeur

Le serveur MCP de l'éditeur et le panneau **Assistant IA** sont fournis par le plugin officiel **Serveur MCP** (`orbit.mcp`) : l'éditeur de base ne contient ni serveur réseau ni moteur asynchrone, ce qui le garde plus léger. Installez le plugin depuis **Outils > Extensions > Plugins…** (voir le chapitre « Plugins de l'éditeur »). Sans lui, le robot de la barre d'état n'apparaît pas et `--mcp-port` indique que le plugin manque ; le serveur en ligne de commande (plus bas) fonctionne toujours. Le port et le jeton déjà enregistrés sont repris tels quels.

Ouvrez le panneau **Assistant** (menu Outils, Ctrl+Maj+A). Il indique si la connexion est prête et donne l'adresse à renseigner dans votre application d'IA. Tant que l'éditeur est ouvert, l'assistant voit votre projet en direct : ses modifications apparaissent immédiatement et s'annulent avec Ctrl+Z comme les vôtres.

## Ce que l'assistant peut faire

Il peut lister et lire les scènes, créer et renommer des scènes, ajouter et modifier des répliques et des choix, placer des commandes (décors, personnages, musique, sauts, conditions, changements de variables), lire la liste des problèmes et enregistrer. Il ne peut rien faire que l'éditeur refuse : un changement qui casserait l'histoire est rejeté avec une explication, et rien n'est appliqué à moitié.

## Bon à savoir

- Chaque demande de l'assistant est une entrée de votre historique d'annulation, nommée d'après l'outil.
- Si vous modifiez la même étape en même temps, l'assistant est prévenu que l'étape a changé et réessaie au lieu d'écraser votre travail.
- Supprimer une scène ou une étape est signalé comme destructeur, afin que votre application d'IA puisse vous demander avant de le faire.

## Sans l'éditeur

Un serveur en ligne de commande (`orbit mcp --project mon_jeu`) offre les mêmes outils quand l'éditeur est fermé. La documentation technique des outils se trouve dans `docs/mcp.md` du dépôt OrbitEngine.
