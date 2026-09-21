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
| `e` | gomme collée / reprise de l’encre |
| `shift+e` | gomme zone |
| `l` `t` `i` | lasso, texte, image |
| `[` `]` `1-9` | épaisseur, encre |
| `m` | cycle papier (vierge, ligné, quadrillé, pointé, millimétré, ardoise) |
| doigt (mode stylet) | panorama |
| espace + glisser | panorama |
| deux doigts | panorama · tap = annuler |
| `+` `−` · boutons · pincement · deux doigts sur le pad · ctrl + molette | zoom |
| coins / `0` / clic sur le % | taille écran |
| bouton du stylet (tenir) | gomme |
| clic bouton en l’air | dernier feutre ↔ gomme |
| 2ᵉ bouton (tenir) | lasso |
| clic droit | gomme |
| shift en relâchant un trait | ligne / cercle / rectangle |
| `ctrl+z` `ctrl+e` `ctrl+shift+e` | undo, PNG, PDF |

Trois modes dans la trousse (puits à côté des plumes) : **stylet** (défaut — le doigt pousse la feuille), **main** (le doigt écrit), **chiffon** (le doigt gomme). La paume est ignorée dès que le stylet est en proximité.

La gomme de la trousse : tap = coller / recoller l’encre ; appui long = trait ↔ zone.

Sur l’étagère, une **fiche** (page arrachée) rappelle ces gestes — tap pour la replier.

Premier lancement : cahiers *Mentalité* et *Direction artistique*.
