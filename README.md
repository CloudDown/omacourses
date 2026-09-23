# Cahier

Stylus notes, like Samsung Notes / Apple Notes, sitting on an **Omarchy lectern**.

Two objects: the **shelf** (notebook spines) and the **page** (paper, pencil case, ruler). Chrome follows the active Omarchy theme; paper stays paper.

Local data only — `~/.local/share/omacourses`. No account, no sync.

![Shelf — notebook spines](docs/screens/etagere.png)

![Lined sketch — Mindset](docs/screens/feuille.png)

![Graph paper — Art direction](docs/screens/millimetre.png)

## Run

```bash
cargo run --release
cahier --help
cahier --data-dir
```

Binary: `cahier`. Data: `~/.local/share/omacourses` (or `$CAHIER_DATA`).

```bash
CAHIER_OPEN=Mindset cahier   # open a notebook by title
```

Theme read from `omarchy theme current` / `colors.toml`.

## Gestures

The lectern follows the stylus: in range, the hand pans and the nib writes. Put it down, the mouse writes again.

| | Mouse | Stylus in range |
|---|---|---|
| Writes | mouse | stylus |
| Pan | space + drag, middle-click, trackpad | finger, two-finger drag |
| Eraser | right-click hold (last kind), `e` / `shift+e` | stylus button hold, air-click to toggle |
| Undo | `ctrl+z` | two-finger tap |
| Zoom | pinch, two-finger trackpad, `+` `−` | pinch |

| | |
|---|---|
| `p` `b` `c` `h` | felt-tip, fountain pen, pencil, highlighter |
| `e` / `shift+e` | stroke (whole stroke) / area (under cursor) |
| `l` `t` `i` | lasso, text, image |
| `[` `]` `1-9` | thickness, ink |
| `m` | cycle paper (blank, lined, grid, dotted, graph, slate) |
| corners / `0` / click on % | fit to screen |
| hold the pen still ~1s | line / arrow / triangle / square / diamond / ellipse / circle |
| `ctrl+e` `ctrl+shift+e` | PNG, PDF |

First launch: notebooks *Mindset* and *Art direction*.
