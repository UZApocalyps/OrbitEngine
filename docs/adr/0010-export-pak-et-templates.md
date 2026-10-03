# ADR 0010 : export des jeux, pak d'assets et templates de player

Statut : accepté (M4, partie « export »).

## Contexte

Un auteur no-code doit pouvoir produire un jeu pour Windows, Linux et le web **sans installer Rust** (ADR 0009 : plugins compilés statiquement, pas de recompilation chez l'auteur). Le web impose en plus un chargement paresseux (un jeu de plusieurs centaines de Mo ne se télécharge pas avant le premier écran) et un wasm petit (10 Mo au début du jalon).

## Décisions

### 1. Un export = un player précompilé (template) + un pak

* Le **player** est compilé une fois (`cargo xtask templates`, ou la CI), sans la feature `showcase` (démo codée en dur, galerie d'UI), et rangé dans un dossier `templates/` :
  `windows/orbit-player.exe`, `linux/orbit-player`, `web/orbit_player.js` + `web/orbit_player_bg.wasm`.
* L'éditeur (installé ou portable) les cherche, dans l'ordre : dossier choisi dans le panneau Export, variable `ORBIT_TEMPLATES`, `templates/` à côté de son exécutable, `../templates` (arbre de build : `target/templates`). L'installeur livre `templates/` (`packaging/templates`, rempli par la CI).
* Un export **colle le pak à une copie du template** : `[exe][pak][longueur du pak u64][« ORBITEND »]`. Le player retrouve son jeu en lisant la fin de son propre exécutable, sinon `game.pak` à côté, sinon `--pak <fichier>`. Un seul fichier à distribuer, qu'on peut renommer.
* **Windows** : zip (`<Jeu>/<Jeu>.exe`, `README.txt`, licence d'Inter). **Linux** : tar.gz (`<Jeu>/<id>`, mode 0755). **Web** : un dossier (voir 4).
* **Icône de l'exe Windows** : non modifiée (celle d'OrbitEngine). Réécrire les ressources d'un exécutable compilé demande `UpdateResource` (Win32, `unsafe`, interdit par les lints du workspace) ou un compilateur de ressources que l'auteur n'a pas. L'icône **de la fenêtre** (barre de titre, barre des tâches pendant le jeu) est bien celle du jeu : le PNG est dans le pak. Dette documentée.
* **Cross-compilation** : le template Linux n'est pas produit depuis Windows (il faut l'éditeur de liens et la glibc de la cible) ; il vient de la CI (runner Ubuntu), comme le template web. `cargo xtask templates --target <triple>` essaie, et dit pourquoi il échoue.

### 2. Le pak : conteneur simple, versionné, sans I/O (`orbit_pak`)

```
"ORBITPAK" | version u32 | taille de l'index u32 | taille des données u64 | index (postcard) | données
```

* L'**index** (métadonnées du jeu + liste `chemin, offset, longueur stockée, longueur d'origine, codec, CRC-32`) précède les données : un lecteur web le récupère seul, puis lit chaque entrée avec une requête HTTP `Range`.
* **Compression optionnelle par entrée** : LZ4 (`lz4_flex`, Rust pur, MIT, compile en wasm), gardée seulement si elle gagne 10 % ; PNG / OGG / JPEG restent bruts. zstd a été écarté : `zstd-sys` est du C, pas de wasm32-unknown-unknown sans toolchain C. Le web ajoute brotli / gzip **sur les fichiers de la page** (wasm, js, html), pas sur le pak.
* **Versionné** : une version plus récente que celle du lecteur est refusée avec un message (jamais devinée). Chaque entrée porte un CRC-32 : un téléchargement tronqué ou un fichier abîmé est une erreur d'asset, pas un crash.
* **Uniquement les assets référencés** (par un nœud ou par une expression de personnage, comme `asset_users`) ; les assets déclarés mais inutilisés et les fichiers de `assets/` hors manifeste donnent un avertissement et ne sont pas exportés.
* Le pak contient aussi le **pack de jeu** (`game.pack`) et les polices du moteur (`engine/assets/fonts/...`) : le jeu exporté n'a aucun chemin du dépôt.

### 3. Le pack de jeu : l'histoire compilée en postcard

`GamePack { info (nom, id, version, langue par défaut), story, translations }`, précédé de `ORBITGAM` + version. `orbit build` valide d'abord (**les erreurs bloquent**), puis sérialise : pas de RON à parser au démarrage, traductions embarquées. Le runtime accepte les deux (`parse_story` reconnaît la magic) : le mode développement (RON + fichiers à plat) est inchangé.

Piège réglé : postcard n'est pas auto-descriptif, donc `skip_serializing_if` est interdit sur le chemin binaire. `Character::default_position` était la seule occurrence ; `Serialize` y est écrit à la main (champ omis seulement dans les formats lisibles). Un test garde l'aller-retour de la démo.

`id` du jeu (profil d'export) = nom du dossier des sauvegardes natives et préfixe `localStorage`. Ne pas le changer après publication.

### 4. Web : chargement paresseux et page

* Le player lit `game.pak` par `Range` : l'index d'abord (64 Kio demandés d'un coup, suffisant en pratique), puis chaque asset à la demande. Un serveur qui ignore `Range` (réponse 200) marche quand même : le pak est alors téléchargé une fois et servi depuis la mémoire.
* **Par scène** : au démarrage, seuls les assets de la première scène et l'image du menu sont chargés. Pendant le jeu, ceux de la scène affichée sont exigés et ceux des scènes atteignables par un saut (`JumpScene`, `Jump` vers un autre label) sont **préchargés en tâche de fond**. Si la scène affichée attend encore un asset (réseau lent, chargement d'une sauvegarde dans une autre scène), le jeu est suspendu derrière un voile « Chargement » ; rien n'est jamais évincé (un retour en arrière est instantané). Natif : le pak est lu entièrement avant le départ, comme avant (disque local).
* La page (`index.html`) a un loader avec **vraie** barre de progression (octets du wasm lus / taille connue à l'export : la compression du serveur fausse `Content-Length`), le logo `assets/brand`, l'icône et le titre du jeu, un message d'erreur lisible. Les URL portent `?v=<crc>` : un nouvel export ne sert jamais un vieux wasm.
* Fichiers précompressés `.br` / `.gz` (profil : `web_precompress`) pour les hôtes qui les servent ; `HOSTING.txt` explique les exigences (Range, `application/wasm`).
* `orbit export web --serve` et le bouton « Tester dans le navigateur » de l'éditeur lancent le même petit serveur (`orbit_export::serve`, loopback, `Range`, `.br`/`.gz`).

### 5. Taille du wasm : budget en CI

Mesures (`orbit_player_bg.wasm`, même code) :

| | octets | brotli (q9) |
|---|---|---|
| avant : profil `release`, sans wasm-opt | 10 167 000 | 2 257 963 |
| profil `release-wasm` (opt-level z, LTO fat, 1 CGU, panic=abort, strip) + wasm-bindgen sans sections de noms | 4 735 377 | |
| + `wasm-opt -Oz` (binaryen 116) | **4 426 429** (−56,5 %) | **1 523 797** (q11 à l'export : 1 360 132) |

Le player web est construit sans la feature `showcase`. **Budget** : `xtask/src/templates.rs` (`WASM_BUDGET_BYTES` = 4 800 000, `WASM_BUDGET_BROTLI_BYTES` = 1 650 000, ~8 % de marge) ; `cargo xtask wasm-budget` / `templates --check-budget` échoue au-dessus (job CI `templates`). On relève le seuil seulement pour une croissance voulue, avec une ligne ici ; jamais pour faire taire une régression.

wasm-opt vient de `cargo install wasm-opt --locked` (crate Apache-2.0, binaryen embarqué) ou d'une release binaryen sur le `PATH` / `--wasm-opt`. **Dégradation propre** : absent, `cargo xtask templates` construit quand même (wasm ~7 % plus gros) et dit comment l'installer.

### 6. Web : cycle de vie, plein écran, DPI, stockage

* `visibilitychange`, `blur`, `focus` et `pagehide` sont remontés par `orbit_platform` (`App::visibility_changed`, `Visibility::{Active, Unfocused, Hidden}`) : sans le focus l'audio est en pause (musique en pause, bus master coupé) ; onglet caché, plus aucun rendu **et une autosave** (`Shell::on_exit`) : un onglet caché peut être tué à tout moment (dette M2 réglée).
* **Plein écran** : l'API du navigateur (via winit), bouton des préférences ou F11 ; un refus (pas de geste utilisateur) est ignoré, et on ne quitte pas un plein écran qui n'existe pas (le navigateur rejette la promesse sinon).
* **DPI** : winit dimensionne le buffer de la canvas en `devicePixelRatio` ; pas de code propre.
* **Stockage : `localStorage` conservé.** Une sauvegarde pèse 10 à 60 Kio (état + miniature PNG 320x180, en base64) : une vingtaine de slots tient dans ~1 Mo des ~5 Mo de l'origine, et `LocalStorage` renvoie déjà `StorageError::QuotaExceeded` (la sauvegarde échoue avec une notification, jamais un crash) quand le quota est atteint. IndexedDB (asynchrone, donc une couche de plus dans `Storage`) ne se justifie que si des jeux avec beaucoup de slots ou des miniatures plus grosses butent sur le quota : à rouvrir sur un cas réel.

## Conséquences

* `orbit_export` (sans egui) est l'unique implémentation : CLI (`orbit build`, `orbit export`), panneau Export de l'éditeur (fil de travail + `Waker`, progression, journal, annulation) et outils MCP `build_project` / `export_game`. Le player ne dépend ni de lui ni de l'éditeur (`check-arch`).
* Les profils d'export vivent dans `orbit.project.ron` (`export_profiles`, champ optionnel : les manifestes existants se chargent sans changement).
* **GamePack version 5** (ADR 0013, addendum M5B-07) : `Motion::Rigged` prend une charge utile, donc la disposition change ; la version 5 n'est écrite que pour un jeu qui a une expression riggée (le rig `.rig.ron` et les images qu'il dessine sont des entrées ordinaires du pak). Les versions 1 à 4 restent lisibles (l'ancienne variante unitaire se lit comme une image fixe, fixture `pak_v3_rigged`).
* Dettes : voir ROADMAP (M4, export) : icône de l'exe Windows, template Linux uniquement par la CI, pas de zip du bundle web, export MCP bloquant le fil de l'éditeur, pas de cache incrémental du pak, pas de signature de code ni de macOS.
