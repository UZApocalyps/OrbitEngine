# Studio IA

Le Studio IA construit un visual novel complet à partir de quelques lignes : vous décrivez le jeu, un modèle de langage de votre choix écrit l'histoire, un générateur d'images dessine les décors et les personnages, et le studio intègre le tout à votre projet. Chaque étape peut être relue, modifiée et relancée.

Le Studio IA est un **plugin de l'éditeur** : l'éditeur est livré sans lui. Compilez-le avec `cargo xtask plugins` et installez `orbit.ai_studio-….orbitplugin` depuis **Outils > Extensions > Plugins…**, puis redémarrez l'éditeur (voir *Plugins de l'éditeur*). Sa page de réglages dans le gestionnaire de plugins contient les réglages des fournisseurs.

Ouvrez-le depuis **Outils > Studio IA**. Il travaille sur le projet ouvert.

Le studio fait partie de l'éditeur uniquement : rien de lui n'est jamais inclus dans un jeu exporté.

## Les cinq étapes

1. **Pitch** : votre texte de base (prémisse, personnages, ambiance...) et des options : genre, ton, langue des dialogues, nombre de scènes, de personnages et de fins, style graphique, répliques par scène.
2. **Bible** : le modèle écrit un plan structuré (titre, synopsis, personnages avec personnalité et apparence, lieux, scènes avec leurs embranchements et leurs fins, variables). Le studio le vérifie ; s'il y a une erreur (une scène qui renvoie à un lieu inconnu, aucune fin...), les erreurs sont renvoyées au modèle qui se corrige. Vous pouvez modifier la bible à la main (JSON) puis l'appliquer.
3. **Script** : chaque scène est écrite dans le langage de script du moteur (voir *Vue texte*), une scène par requête pour que les longues histoires tiennent dans la mémoire du modèle. Chaque scène est vérifiée par le même analyseur que la vue texte ; les erreurs, avec leur numéro de ligne, repartent au modèle pour réparation. Une scène encore fausse après les réparations permises est conservée, marquée d'un avertissement, pour que vous la corrigiez (**Vérifier et enregistrer**). **Régénérer** réécrit une scène.
4. **Images** : un décor par lieu, et un sprite par personnage et par expression réellement utilisée dans le script (`neutral`, `happy`, `sad`, `angry`, `surprised`, `embarrassed`, `scared`, `thinking`). Chaque prompt est modifiable ; **Refaire** régénère une image. Une image qui échoue est remplacée par une image provisoire : le jeu fonctionne toujours. Les fichiers vont dans `assets/ai/backgrounds/<lieu>.png` et `assets/ai/characters/<personnage>/<expression>.png`.
5. **Intégration** : crée les personnages et leurs expressions, les décors, les variables et les scènes, avec les mêmes opérations que l'assistant MCP, en **une seule** étape annulable (Ctrl+Z retire tout). L'histoire est validée et l'aperçu s'ouvre. Cochez *Remplacer les scènes* pour que le jeu commence par l'histoire générée (par défaut pour un projet vide) ; sinon les scènes sont ajoutées à la suite.

Les étapes longues tournent en arrière-plan avec une barre de progression ; **Annuler** les arrête. L'état est enregistré après chaque scène et chaque image dans `.orbit/ai/studio.json` dans le projet : relancer une étape reprend là où elle s'était arrêtée, même après avoir fermé l'éditeur.

## Modèle de texte

Tout serveur avec une API `chat/completions` compatible OpenAI convient : Ollama, LM Studio, serveur llama.cpp, KoboldCpp, vLLM, OpenRouter, OpenAI... Vous choisissez le modèle ; le studio n'ajoute aucun filtrage. Réglez-le dans l'onglet **Fournisseurs** : URL (jusqu'à `/v1`), modèle, clé (vide pour un serveur local), température, jetons max et nombre de réparations permises.

### Exemple : Ollama (local)

1. Installez Ollama, puis téléchargez un modèle : `ollama pull llama3.1` (n'importe quel modèle ; à partir de 12B les scripts sont nettement meilleurs).
2. Dans **Fournisseurs**, URL `http://127.0.0.1:11434/v1` (l'URL par défaut `http://localhost:11434/v1` convient aussi), modèle `llama3.1`, pas de clé. Une URL nue `http://127.0.0.1:11434` reçoit son `/v1` automatiquement. Si la connexion échoue, utilisez `127.0.0.1` plutôt que `localhost` : sur certains systèmes `localhost` désigne l'IPv6 (`::1`) alors qu'Ollama n'écoute qu'en IPv4 (le studio essaie toutes les adresses, mais un pare-feu ou un proxy peut quand même gêner).
3. **Tester la connexion / lister les modèles** affiche les modèles du serveur. Si le serveur répond *model not found*, le modèle n'est pas téléchargé : lancez `ollama pull <modèle>` dans un terminal et testez à nouveau.

Les modèles « à raisonnement » conviennent : leur section `<think>` est retirée des réponses.

## Génération d'images

Choisissez le moteur dans **Fournisseurs** :

- **Placeholder** : pas de génération, des images provisoires colorées (à remplacer plus tard dans la bibliothèque).
- **ComfyUI** : exécute un workflow au *format API* (dans ComfyUI : *Workflow > Export (API)*). Celui par défaut est un txt2img simple. Les textes peuvent contenir `{{prompt}}`, `{{negative}}`, `{{model}}`, `{{sampler}}`, `{{scheduler}}` ; une valeur écrite exactement `"{{seed}}"`, `"{{width}}"`, `"{{height}}"`, `"{{steps}}"` ou `"{{cfg}}"` devient un nombre. Les nœuds LoRA, le VAE et le clip skip sont ajoutés automatiquement après le chargeur de checkpoint.
- **AUTOMATIC1111 / Forge** : `/sdapi/v1/txt2img` (lancez la web UI avec `--api`).
- **Compatible OpenAI** : `images/generations`.

Réglages communs : préfixe de style (ou le *style graphique* du pitch), prompt négatif, tailles des décors et des sprites, graine de base, images générées en parallèle. Chaque personnage dérive une graine fixe de la graine de base et garde ainsi la même apparence d'une expression à l'autre. Les sprites sont demandés sur fond blanc uni, que le studio rend transparent (option).

### Exemple : ComfyUI (local)

1. Lancez ComfyUI (adresse par défaut `http://127.0.0.1:8188`).
2. Moteur *ComfyUI*, URL `http://127.0.0.1:8188`.
3. Dans **Modèles SD**, cliquez **Lire les modèles du moteur** et choisissez un checkpoint.

## Modèles Stable Diffusion (onglet Modèles SD)

- **Famille de modèle** : *SD 1.5*, *SDXL*, *Pony*, *Illustrious / NoobAI* règlent des tailles adaptées, les tags de qualité (`score_9, score_8_up...` pour Pony), le prompt négatif et le clip skip. Chaque valeur reste modifiable.
- **Checkpoint, VAE, sampler, scheduler** : choisissez dans les listes déroulantes avec recherche (tapez pour filtrer), ou tapez un nom. Les listes fusionnent deux sources, marquées *disque* et *moteur* : les fichiers trouvés dans les dossiers de modèles (voir *Dossiers de modèles* plus bas) et ce que signale le moteur (A1111 / Forge : checkpoints, LoRA, VAE, embeddings, samplers, schedulers ; ComfyUI : les choix de ses nœuds de chargement et de sampler). Un nom présent dans aucune liste est conservé tel quel, avec un avertissement. **Rescanner le disque** relit les dossiers. Les LoRA s'ajoutent aussi depuis une liste (**Choisir une LoRA...**), et **Insérer un embedding** en ajoute un à un prompt. Les noms de sampler au format A1111 (`DPM++ 2M Karras`) sont traduits pour ComfyUI.
- **LoRA pour toutes les images** : nom, poids et mots déclencheurs. Avec A1111 le studio écrit `<lora:nom:poids>` dans le prompt ; avec ComfyUI il ajoute des nœuds `LoraLoader`. Les mots déclencheurs sont ajoutés aux prompts automatiquement.
- **Embeddings** : écrivez leur nom dans les tags de qualité, le préfixe de style ou le prompt négatif.
- **Cohérence des personnages** (onglet Images) : par personnage, une graine fixe et / ou une LoRA de personnage (par exemple entraînée sur ce personnage).

### Dossiers de modèles

Le studio peut lire vos modèles directement sur le disque, même quand le moteur ne tourne pas. Sous **Dossiers de téléchargement** (onglet Modèles SD), donnez les quatre dossiers de votre moteur (checkpoints, LoRA, embeddings, VAE), ou laissez le studio les trouver : saisissez le **Dossier de ComfyUI** (le dossier d'installation ; la version portable avec son dossier `ComfyUI` interne convient aussi) ou le **Dossier de A1111 / Forge** (le dossier `stable-diffusion-webui`) et cliquez sur **Détecter les dossiers de modèles**. Un message indique *Dossiers de modèles trouvés.* ou *Aucun dossier models trouvé ici.* Les mêmes dossiers servent de destination aux téléchargements Civitai.

## Civitai (facultatif)

Dans l'onglet **Modèles SD**, cherchez des modèles Civitai par nom et par type (Checkpoint, LORA, TextualInversion, VAE). Les résultats montrent le nom, le type, le modèle de base, un aperçu et les mots déclencheurs.

- **Dossiers de téléchargement** : indiquez les dossiers de modèles de votre moteur, par ex. `ComfyUI/models/checkpoints`, `ComfyUI/models/loras`, `ComfyUI/models/embeddings`, `ComfyUI/models/vae`, ou pour A1111 / Forge `models/Stable-diffusion`, `models/Lora`, `embeddings`, `models/VAE`.
- **Télécharger** enregistre le fichier dans ce dossier, avec progression et annulation dans la barre d'état. Le fichier est vérifié avec l'empreinte SHA-256 publiée par Civitai (un téléchargement corrompu est supprimé) et ignoré s'il est déjà présent. Le moteur est ensuite invité à relire ses modèles (A1111 / Forge ; ComfyUI voit les nouveaux fichiers tout seul).
- **Utiliser cette LoRA** l'ajoute à la liste avec ses mots déclencheurs.
- **Jeton d'API** : certains modèles en demandent un (paramètres du compte Civitai). Il est enregistré avec vos réglages utilisateur.
- **Afficher NSFW** : un filtre d'affichage des résultats et des aperçus, désactivé par défaut.

Respectez la licence de chaque modèle téléchargé.

## Confidentialité et fichiers

- Les réglages des fournisseurs, les clés et le jeton Civitai sont enregistrés dans votre dossier de configuration utilisateur (`ai_studio.ron` à côté du `config.ron` de l'éditeur), jamais dans le projet.
- L'état du studio est dans le dossier `.orbit/ai/` du projet, que les exports n'incluent jamais.
- Les adresses `http://` (serveurs locaux) sont contactées directement. Les adresses `https://` (OpenRouter, OpenAI, Civitai) passent par le programme système `curl`, présent sur Windows 10 et suivants, macOS et Linux.

## Pour les assistants IA (MCP)

Quand le serveur MCP de l'éditeur est actif, un assistant dispose aussi des outils du studio : `orbit.ai_studio.dsl_guide` (la grammaire du script, un exemple, le schéma de la bible et le nommage des fichiers), `orbit.ai_studio.check_bible` et `orbit.ai_studio.check_scene` (les mêmes vérifications que l'assistant de création). Il écrit ensuite les scènes avec `apply_dsl` et les autres outils intégrés.
