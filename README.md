# Cahier

Notes au stylet, comme Samsung Notes / Apple Notes, posées sur un **pupitre Omarchy**.

Deux objets : l’**étagère** (dos de cahiers) et la **feuille** (papier, trousse, règle). Le chrome suit le thème Omarchy actif ; le papier reste du papier.

Données locales uniquement — `~/.local/share/omacourses`. Pas de compte, pas de sync.

![Étagère — dos de cahiers](docs/screens/etagere.png)

![Feuille lignée — Mentalité](docs/screens/feuille.png)

![Papier millimétré — Direction artistique](docs/screens/millimetre.png)

## Lancer

```bash
cargo run --release
cahier --help
cahier --data-dir
```

Binaire : `cahier`. Données : `~/.local/share/omacourses` (ou `$CAHIER_DATA`).

```bash
CAHIER_OPEN=Mentalité cahier   # ouvre un cahier par titre
```

Thème lu depuis `omarchy theme current` / `colors.toml`.

## Gestes

| | |
|---|---|
| `p` `b` `c` `h` | feutre, plume, crayon, surligneur |
| `e` / `shift+e` | gomme trait / zone |
| `l` `t` `i` | lasso, texte, image |
| `[` `]` `1-9` | épaisseur, encre |
| `m` | cycle papier (vierge, ligné, quadrillé, pointé, millimétré, ardoise) |
| espace + glisser, deux doigts | panorama |
| ctrl + molette | zoom |
| clic droit | gomme |
| shift en relâchant un trait | ligne / cercle / rectangle |
| `ctrl+z` `ctrl+e` `ctrl+shift+e` | undo, PNG, PDF |

Stylet : pression via événements tactiles si le compositeur les envoie ; sinon la plume simule la pression par la vitesse. Mode **stylet** dans la trousse ignore la paume.

Premier lancement : cahiers *Mentalité* et *Direction artistique*.
