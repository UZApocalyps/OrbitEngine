# À faire plus tard

Idées validées mais hors des jalons en cours. À replanifier dans la roadmap le moment venu.

## Dialogues en bulles positionnables
- Mode d'affichage alternatif à la textbox classique : chaque réplique est rendue dans une **bulle** (style BD / conversation) plutôt que dans la boîte du bas.
- Le créateur positionne les bulles depuis l'éditeur, sans code : par personnage (bulle ancrée au personnage, suit sa position) ou par réplique (position libre surchargée).
- Réglages : forme et style de la bulle (thème), queue orientée vers le locuteur, taille max / retour à la ligne auto, empilement ou remplacement des bulles précédentes (effet conversation à deux).
- Doit rester compatible avec : machine à écrire, historique, rollback/skip (état des bulles déclaratif), localisation (taille variable du texte), et le MCP (outils de placement).
- Dépend de : `orbit_ui` (thèmes), éditeur M3 (placement visuel), M5 (animation d'apparition des bulles).
