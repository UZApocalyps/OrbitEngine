# Scènes 3D

Un visual novel peut utiliser une scène 3D comme décor : une pièce où l'on peut regarder autour, une rue avec une caméra mobile. Les personnages 2D et la boîte de dialogue sont dessinés par-dessus.

## Construire une scène 3D

Ouvrez l'**éditeur de scène 3D**. Il comporte trois panneaux :

- **Vue 3D** : la scène telle que la voit la caméra. Tournez, déplacez et zoomez pour regarder autour.
- **Hiérarchie 3D** : la liste des objets de la scène (modèles, lumières, caméras).
- **Inspecteur 3D** : les réglages de l'objet sélectionné (position, rotation, taille, couleur, intensité de la lumière).

Créez une nouvelle scène, importez des modèles 3D, placez des lumières et ajoutez une ou plusieurs caméras nommées. Enregistrez la scène : elle devient un fichier de votre projet (`.scene3d.ron`) que vous pouvez réutiliser dans l'histoire.

## L'utiliser dans l'histoire

Trois étapes pilotent la 3D dans l'histoire :

- **Afficher une scène 3D** : affiche une scène 3D comme décor, avec une caméra et une transition facultative.
- **Caméra** : passe à une autre caméra nommée, par exemple pour couper ou aller sur un gros plan.
- **Masquer la scène 3D** : revient à un décor 2D normal.

Passer de la 2D à la 3D et inversement utilise les mêmes transitions que n'importe quel décor.

## Conseils

- Commencez avec une lumière et une caméra, puis ajoutez des détails.
- Testez dans le panneau Preview : il utilise le même moteur de rendu que le jeu exporté.
