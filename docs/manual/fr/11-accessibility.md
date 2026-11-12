# Accessibilité

Les jeux OrbitEngine sont conçus pour être jouables par le plus grand nombre. Ce chapitre liste ce qui est disponible aujourd'hui.

## Lecteur d'écran

Les joueurs qui utilisent un lecteur d'écran peuvent activer l'option **lecteur d'écran** dans l'écran des préférences. Le jeu décrit alors ses écrans, boutons, curseurs et dialogues au lecteur (sur ordinateur via le service d'accessibilité du système, sur le web via une structure de page cachée placée à côté du jeu). Les nouvelles répliques sont annoncées au fur et à mesure.

Pour que cela fonctionne bien, donnez des textes parlants à vos propres écrans : un bouton qui porte des mots est annoncé avec ces mots, un bouton qui n'a qu'une image ne l'est pas. Gardez-le en tête dans l'éditeur d'interface.

## Clavier et manette

Les menus et le jeu se jouent sans souris : le clavier et les manettes sont pris en charge, et les boutons de manette associés aux actions du jeu sont conservés dans les préférences du joueur. Sur téléphone ou tablette, le jeu répond au toucher.

## Réglages de confort

L'écran des préférences permet de régler la vitesse du texte, le délai de l'avance automatique, le mode de saut, le volume de chaque catégorie de son et le plein écran.

## Taille du texte, contraste élevé et animations réduites

Trois autres options se trouvent dans l'écran des préférences. Toutes sont désactivées par défaut : un jeu se présente et se joue exactement comme vous l'avez conçu tant qu'un joueur ne les change pas. Elles sont enregistrées avec les autres préférences du joueur.

- **Taille du texte** agrandit le dialogue, les choix et le texte des menus de 100 % à 160 %, par pas de 10 %. La boîte de dialogue grandit vers le haut et les boutons de choix grandissent avec le texte : les lignes ne sortent jamais de leur boîte.
- **Contraste élevé** remplace les couleurs de votre thème par du texte blanc sur fond noir, des surbrillances jaunes et des bordures épaisses. Vos tailles, polices et mises en page sont conservées, ainsi que les noms de vos styles. Le mode s'applique par-dessus votre thème : il ne modifie jamais vos fichiers.
- **Réduire les animations** supprime les animations. Les transitions entre fonds deviennent des coupes, et les effets comme les déplacements, secousses, zooms et flashs, ainsi que les timelines, sautent directement à leur état final : la scène est exactement la même que si l'animation s'était déroulée. L'histoire ne les attend pas. Les pauses chronométrées que vous avez écrites avec une attente sont conservées. Les particules (pluie, neige...) ne sont pas dessinées. Le marqueur « continuer » cesse de clignoter.

Votre propre écran de préférences peut proposer les mêmes options : dans l'éditeur d'interface, liez un curseur à **Taille du texte**, et des interrupteurs à **Contraste élevé**, **Réduire les animations** et **Lecteur d'écran**. L'écran de préférences par défaut les contient déjà. Un écran créé avant l'arrivée de ces options continue de fonctionner ; il ne les affiche simplement pas tant que vous n'ajoutez pas les contrôles.

### Vérifier votre thème

L'éditeur d'interface signale les couleurs de thème difficiles à lire : dans l'onglet Thème et dans la liste des problèmes, chaque paire de couleurs texte/fond sous le contraste WCAG AA (4,5:1 pour le texte, 3:1 pour les éléments des curseurs et interrupteurs) est listée avec son ratio. Un avertissement n'empêche pas d'exporter votre jeu. Les joueurs qui ont besoin de plus de contraste peuvent toujours activer le mode contraste élevé, qui passe toujours la vérification.

## Notifications et point & click

- Les **notifications** (succès, messages du moteur et étape Notification) sont lues par le lecteur d'écran quand elles apparaissent. Avec **Réduire les animations** activé, une notification qui glisse apparaît en fondu. Leur comportement pendant le passage rapide du texte se règle dans l'onglet *Notifications* de l'éditeur d'interface.
- Les **zones cliquables** sont accessibles sans souris : au clavier (Tab, flèches, chiffres) et à la manette. La touche **V** (et un bouton de manette, Select / Vue par défaut) entoure toutes les zones d'un coup, pour que personne n'ait à les chercher. Une zone grisée n'est pas encore cliquable.

## Ce que vous pouvez faire en tant qu'auteur

- Rédigez des choix clairs, sans vous reposer uniquement sur des couleurs ou des positions.
- Ne comptez pas sur le son seul pour donner une information ; écrivez-la aussi dans une réplique.
- Testez vos écrans personnalisés au clavier uniquement.
