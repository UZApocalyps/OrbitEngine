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

## Travailler avec

- Modifiez le texte librement, puis appuyez sur **Appliquer** (Ctrl+Entrée). L'ensemble du changement forme une seule étape d'annulation.
- Les erreurs sont signalées sur la ligne concernée et dans le panneau Problèmes. Rien n'est appliqué tant que le texte contient des erreurs.
- **Recharger** abandonne votre saisie et réaffiche l'histoire actuelle.
- De petites marques discrètes en fin de ligne (`#@...`) permettent à l'éditeur de reconnaître chaque étape ; laissez-les en place quand vous déplacez des lignes.

Le texte n'est qu'une vue : l'histoire elle-même n'est pas stockée sous forme de texte, vous pouvez donc passer de la Vue texte aux autres éditeurs à tout moment.
