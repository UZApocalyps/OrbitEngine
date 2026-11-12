# Tutoriel : d'autres façons de démarrer un projet

On ne commence pas toujours devant une page blanche. Si vous avez déjà un jeu Ren'Py, l'éditeur peut le transformer en projet pour vous. Si vous n'avez qu'une idée, le Studio IA peut écrire une première version du jeu à partir de quelques lignes de texte. Dans les deux cas, vous obtenez un projet à terminer à la main.

## Partie 1. Importer un jeu Ren'Py

### Objectif

Convertir un jeu Ren'Py existant en projet OrbitEngine, lire le rapport d'import, et corriger ce que l'importeur n'a pas pu convertir.

### Avant de commencer

- Le dossier du jeu Ren'Py : celui qui contient `game/`, ou `game/` lui-même. L'importeur lit les fichiers source `.rpy`, les scripts doivent donc être présents (un jeu livré uniquement en archives compilées ne peut pas être importé). Le jeu d'origine n'est jamais modifié.
- Un dossier vide, ou un endroit où un nouveau dossier peut être créé, pour le nouveau projet.
- Des images PNG ou JPEG et des sons OGG ou WAV. Les images WebP et AVIF et l'audio MP3 ou Opus ne sont pas importés : convertissez-les d'abord.

### Étapes

1. Choisissez **Fichier > Import, export et traductions > Importer depuis Ren'Py…**.
2. Choisissez le dossier du jeu Ren'Py, puis le dossier qui contiendra le nouveau projet. La tâche **Import du jeu Ren'Py** s'exécute en arrière-plan, avec une barre de progression et un bouton **Annuler cette tâche** dans la barre d'état.
3. Quand elle est terminée, le projet s'ouvre et la fenêtre **Rapport d'import Ren'Py** affiche un résumé : ce qui a été converti, ce qui a été approché, et chaque instruction non convertie avec son fichier et sa ligne. Le même rapport est enregistré dans le nouveau projet sous `import-report.md`, vous pouvez donc y revenir.
4. Ouvrez le panneau **Problèmes**. Le projet est validé, mais des avertissements signalent des endroits à regarder : une étape jamais atteinte, une scène qui termine la partie, une variable utilisée avant d'être assignée.
5. Jouez le début avec **Preview** (F6) et comparez avec le jeu d'origine. Regardez d'abord les dialogues, les personnages et les décors.
6. Parcourez le rapport de haut en bas et corrigez, dans cet ordre :
7. **Les étapes Script** : le Python qui n'est pas une simple affectation est conservé sous forme d'étape de script commentée. Sélectionnez-la dans le panneau **Scène** et réécrivez-la avec des étapes (**Modifier une variable**, **Condition**) ou en Rhai (voir *Scripts (Rhai)*).
8. **Les conditions converties en Rhai** : ouvrez chaque étape **Condition** que le rapport cite et vérifiez qu'elle se comporte comme l'original.
9. **Les écrans d'interface** : les écrans de dialogue, de menu, de sauvegarde et de préférences de Ren'Py ne sont pas convertis. Construisez les vôtres dans le panneau **Interface** (voir *Tutoriel : personnaliser l'interface du jeu*). Un écran fait d'une image cliquable ou de boutons-images utilisé avec `call screen` est devenu une étape **Zones** (point & click).
10. **Les médias manquants ou non pris en charge** : ajoutez les images et sons convertis avec **Importer** dans le panneau **Assets**, et vérifiez que les expressions et décors que le panneau Problèmes signale comme manquants ont maintenant leurs images.
11. Enregistrez avec Ctrl+S, et exportez quand tout se joue bien.

### Résultat

Un projet qui s'ouvre, se valide et se joue, avec vos scènes, personnages, variables, choix, transitions et traductions, et un rapport qui liste tout ce qu'il reste à faire.

### Pour aller plus loin

- La ligne de commande fait la même chose : `orbit import-renpy <dossier_renpy> <dossier_sortie>` ; le dossier de sortie doit être vide ou absent.
- Les traductions Ren'Py (`tl/<langue>/`) deviennent des traductions du projet (voir *Traductions*).
- Les succès enregistrés avec `achievement.register` deviennent des succès avec leurs étapes de déblocage.

### En cas de problème

- **L'import ne trouve rien** : vérifiez que le dossier choisi contient les fichiers `.rpy` (le dossier `game/`), et pas seulement une archive `.rpa`.
- **Beaucoup de balises de texte ont disparu** : les balises comme `{size}`, `{font}` et `{a}` sont supprimées ; le rapport les compte par balise.
- **Un personnage affiche la mauvaise image** : Ren'Py nomme ses images par convention (`eileen happy.png`). Vérifiez l'expression dans le panneau **Personnages** et choisissez le bon fichier avec **Fichier…**.
- **Un sprite n'est plus au même endroit** : les positions sont converties vers l'ancrage du moteur (bas au centre). Replacez-le dans l'**Éditeur de vue**.

## Partie 2. Générer un jeu avec le Studio IA

### Objectif

Produire un premier jet jouable de visual novel à partir d'un court pitch, avec un modèle de langage et un générateur d'images qui tournent sur votre propre ordinateur. Comptez environ une heure la première fois, surtout à attendre.

### Avant de commencer

- **Le plugin.** Le Studio IA est un plugin d'éditeur : l'éditeur est livré sans lui. Construisez-le depuis le dépôt avec `cargo build --release -p orbit_editor`, puis `cargo xtask plugins`. Cela écrit `orbit.ai_studio-<version>-<système>.orbitplugin` dans le dossier `plugins` de votre dossier cible cargo. Le plugin doit être construit pour la même version de l'éditeur.
- **Un modèle de texte.** Installez Ollama, puis téléchargez un modèle dans un terminal, par exemple `ollama pull llama3.1`. Un modèle d'environ 12 milliards de paramètres ou plus écrit des scripts nettement meilleurs. Tout serveur avec une API `chat/completions` compatible OpenAI convient aussi.
- **Un générateur d'images** (facultatif). Lancez ComfyUI (adresse par défaut `http://127.0.0.1:8188`) avec un checkpoint installé. Sans lui, le studio utilise des images provisoires colorées et le jeu fonctionne quand même.
- Un projet ouvert dans l'éditeur, vide ou non.

### Étapes

1. **Installer le plugin.** Choisissez **Outils > Extensions > Plugins…**, cliquez sur **Installer depuis un fichier…** et choisissez le fichier `.orbitplugin`. Lisez l'avertissement **Installer du code natif ?** : un plugin s'exécute avec les mêmes droits que l'éditeur, n'installez donc que des plugins de confiance. Cliquez sur **Installer**, puis sur **Redémarrer maintenant**.
2. **Ouvrir le studio.** Choisissez **Outils > Studio IA (créer un jeu avec l'IA)…**. Il travaille sur le projet ouvert.
3. **Onglet Fournisseurs, modèle de texte.** Sous **Modèle de texte (compatible OpenAI)**, réglez **URL** sur `http://127.0.0.1:11434/v1` et **Modèle** sur `llama3.1`. Laissez la clé vide. Cliquez sur **Tester la connexion / lister les modèles** : le studio indique combien de modèles sont disponibles. Utilisez `127.0.0.1` plutôt que `localhost` : certains systèmes résolvent `localhost` vers une adresse IPv6 sur laquelle le serveur n'écoute pas.
4. **Onglet Fournisseurs, images.** Réglez **Moteur** sur **ComfyUI** et l'URL sur `http://127.0.0.1:8188`. Pour un premier essai sans génération d'images, choisissez plutôt le moteur provisoire (Placeholder).
5. **Onglet Modèles SD** (ComfyUI ou AUTOMATIC1111 / Forge seulement). Cliquez sur **Lire les modèles du moteur** et choisissez un **Checkpoint**. Choisissez la **Famille de modèle** correspondante (SD 1.5, SDXL, Pony, Illustrious / NoobAI) : elle règle comme il faut les tailles, les étiquettes de qualité et le clip skip.
6. **Onglet 1. Pitch.** Décrivez votre jeu : situation de départ, personnages, ambiance. Réglez le genre, le ton, la langue des dialogues, le nombre de scènes, de personnages et de fins, le style graphique et le nombre de répliques par scène. Commencez petit : trois scènes, deux personnages, deux fins.
7. Cliquez sur **Écrire la bible de l'histoire**. Le modèle écrit un plan structuré ; le studio le vérifie et renvoie toute erreur au modèle pour qu'il se corrige.
8. **Onglet 2. Bible.** Lisez le titre, le synopsis, les personnages, les lieux, les scènes et les variables. Vous pouvez modifier la bible à la main et cliquer sur **Vérifier et appliquer**. Une nouvelle bible efface les scripts et les images qui la suivent.
9. **Onglet 3. Script.** Cliquez sur **Écrire les scènes**. Chaque scène est écrite l'une après l'autre dans le langage de script du moteur et vérifiée comme le fait la Vue texte. Une scène qui a encore des erreurs est conservée avec un avertissement : corrigez-la à la main et cliquez sur **Vérifier et enregistrer**. **Régénérer** réécrit une seule scène.
10. **Onglet 4. Images.** Cliquez sur **Planifier les images** : un décor par lieu, un sprite par personnage et par expression utilisée dans le script. Modifiez les prompts qui ne vous plaisent pas, puis cliquez sur **Générer les images manquantes**. **Refaire** refait une image. Une image en échec est remplacée par une image provisoire pour que le jeu fonctionne toujours.
11. **Onglet 5. Intégration.** Cochez **Remplacer les scènes de l'histoire (sinon les nouvelles scènes sont ajoutées à la suite)** si vous voulez que l'histoire générée devienne le jeu. Cliquez sur **Intégrer au projet**. Le studio crée les personnages, expressions, décors, variables et scènes en une seule étape annulable (Ctrl+Z retire tout), valide l'histoire et ouvre Preview.
12. Jouez-la, ouvrez le panneau **Problèmes**, et terminez le jeu à la main : réécrivez des répliques, remplacez des images dans la bibliothèque, ajoutez de la musique.

### Résultat

Un projet avec des personnages, des décors, un script avec choix et fins, prêt à être peaufiné comme n'importe quel autre projet.

### Pour aller plus loin

- Les longues étapes tournent en arrière-plan avec une barre de progression et un bouton **Annuler**. L'état est enregistré après chaque scène et chaque image dans `.orbit/ai/studio.json` du projet : relancer une étape reprend là où elle s'était arrêtée, même après avoir fermé l'éditeur.
- Téléchargez des checkpoints et des LoRA depuis Civitai dans l'onglet **Modèles SD**, et donnez à un personnage une graine ou une LoRA fixe dans l'onglet Images pour garder son apparence.
- Connectez plutôt un assistant IA (MCP) : il reçoit aussi les outils de vérification du studio (voir *L'assistant MCP*).

### En cas de problème

- **« Tester la connexion » échoue** : vérifiez qu'Ollama tourne (`ollama list` dans un terminal) et que l'URL se termine par `/v1`.
- **Les réponses du modèle sont mauvaises ou coupées** : choisissez un modèle plus grand, augmentez le nombre maximal de jetons, ou autorisez plus de réparations dans l'onglet Fournisseurs.
- **Aucune image n'est générée** : vérifiez que ComfyUI tourne à l'adresse donnée et qu'un checkpoint est choisi ; les images provisoires gardent le jeu jouable en attendant.
- **Le plugin est signalé Incompatible** : il a été construit pour une autre version de l'éditeur ou une autre chaîne d'outils. Reconstruisez-le avec la chaîne d'outils de l'éditeur (voir *Plugins de l'éditeur*).
- **Une scène reste signalée par un avertissement** : ouvrez l'onglet Script, lisez l'erreur avec son numéro de ligne, corrigez la ligne et cliquez sur **Vérifier et enregistrer**.
