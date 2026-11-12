# Vue texte

Le panneau **Vue texte** montre une scène (ou toute l'histoire) sous forme de texte brut qui se lit presque comme un scénario. Elle est facultative : l'éditeur de scène et le Graphe restent la façon principale d'écrire. Certains auteurs préfèrent simplement taper.

## À quoi cela ressemble

```
scene morning
label start
bg "assets/bg_dawn.png" with fade(1.0, "#000000")
"Dawn over the harbour town."
Mira "I almost did not come."
menu
    "Thank her warmly." -> thank
    "Say nothing." -> silent
$ friendship += 1
jump join
```

Une étape par ligne : une réplique de narrateur est simplement le texte entre guillemets, une réplique de personnage commence par le nom du personnage, `menu` introduit des choix, `$` modifie une variable, `jump` va à une étiquette, `bg`, `show`, `music` et `sound` règlent le décor et les sons, et `#` commence un commentaire.

## Les autres étapes

La même logique d'une étape par ligne couvre le reste de l'histoire. Quelques lignes, une par type d'étape (voix, effets, écrans, script, succès, notification, 3D, point & click) :

```
Mira "Bonjour." voice "assets/voice/bonjour.ogg"
shake screen 0.02 over 0.4
zoom 1.2 over 2.0 ease OutQuad wait
open screen "inventory" wait -> used_item
script "set_var(\"or\", 5)" branch win -> victoire on_error oups
unlock "Premiers mots"
notify "Sauvegardé !" icon "assets/ui/cloche.png" style system duration 3.0
scene3d "assets/3d/salle.scene3d.ron" camera "main"
hotspots bg "assets/hall.png"
    spot rect (0.1, 0.2, 0.25, 0.5) "La porte" -> dehors name porte
        do set cle = 1
        do stay
```

Les zones s'écrivent en fractions de l'écran (0 à 1) : `rect (x, y, l, h)`, `ellipse (cx, cy, rx, ry)`, `poly (x, y) (x, y) ...` ou `mask <image> (x, y, l, h) [threshold f] [draw]` pour une zone image. Une zone peut aussi porter `hover`, `sound`, `style`, `enabled <condition>` et `if <condition>`, et ses lignes `do` sont les actions au clic (`set`, `jump`, `scene`, `call`, `return`, `sound`, `notify`, `stay`). Une condition Rhai s'écrit `rhai "..."`. Les particules, les timelines et les fonds en parallaxe ont aussi une forme texte ; la Vue texte les affiche, et le Studio IA utilise le même langage.

## Travailler avec

- Modifiez le texte librement, puis appuyez sur **Appliquer** (Ctrl+Entrée). L'ensemble du changement forme une seule étape d'annulation.
- Les erreurs sont signalées sur la ligne concernée et dans le panneau Problèmes. Rien n'est appliqué tant que le texte contient des erreurs.
- **Recharger** abandonne votre saisie et réaffiche l'histoire actuelle.
- De petites marques discrètes en fin de ligne (`#@...`) permettent à l'éditeur de reconnaître chaque étape ; laissez-les en place quand vous déplacez des lignes.

Le texte n'est qu'une vue : l'histoire elle-même n'est pas stockée sous forme de texte, vous pouvez donc passer de la Vue texte aux autres éditeurs à tout moment.
