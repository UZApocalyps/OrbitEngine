# Éditeur d'interface et splash

Les menus de votre jeu (menu principal, pause, sauvegarde, chargement, préférences) sont des **écrans** que vous pouvez restyler sans code. Ils vivent dans le dossier `ui/` de votre projet. Sans ce dossier, le jeu utilise les écrans par défaut.

## L'éditeur d'interface

Ouvrez le panneau **Interface**. Il comporte trois onglets :

- **Écrans** : choisissez un écran et modifiez-le. Ajoutez des boutons, des textes, des images et des conteneurs, déplacez-les, et réglez l'action d'un bouton (lancer le jeu, ouvrir un autre écran, quitter...). Un aperçu en direct de la vraie interface du jeu est affiché à côté de l'éditeur.
- **Thème** : couleurs, polices et apparence commune à tous les écrans.
- **Splashes** : les écrans affichés au démarrage du jeu.

Le modèle **Démarrage visual novel** contient déjà un jeu complet d'écrans à adapter. Les modifications sont enregistrées dans le dossier `ui/` ; annuler et rétablir fonctionnent comme d'habitude. Le panneau Interface liste les problèmes (par exemple un bouton qui pointe vers un écran inexistant).

## Les splashes

Au démarrage, le jeu affiche d'abord le splash OrbitEngine. Celui-ci est obligatoire et ne peut être ni retiré ni modifié. Ensuite, vous pouvez ajouter vos propres splashes : un texte ou une image de votre projet, avec une durée, des temps d'apparition et de disparition en fondu, une couleur de fond, et la possibilité ou non de le passer. Utilisez **Aperçu des splashes** pour les visionner.
