mod app;
mod camera;
mod document;
mod emoji;
mod export;
mod fonts;
mod ink;
mod library;
mod look;
mod pressure;
mod seed;
mod tablet;
mod undo;

use app::CahierApp;

const HELP: &str = "\
cahier — stylus notes, Omarchy lectern

Usage:
  cahier
  cahier --help
  cahier --version
  cahier --data-dir
  cahier --theme

Opens the lectern. Notebooks live in $CAHIER_DATA
or ~/.local/share/omacourses. Chrome follows
~/.local/state/omarchy/current/theme (same palette as the terminal).

Examples:
  cahier
  CAHIER_DATA=/tmp/cahier cahier
  CAHIER_OPEN=Mindset cahier
  cahier --theme
  cahier --data-dir

Gestures (inside a notebook):
  p felt-tip   b fountain   c pencil   h highlighter
  e stroke eraser   shift+e area eraser
  l lasso   t text   i image
  [ ] thickness   1-9 ink   m paper
  + − zoom   0 / corners = fit to screen
  mouse writes · space pans · right-click = last eraser
  stylus in range: stylus writes · finger pans · two-finger tap = undo
  trackpad: pinch or two fingers (vertical) = zoom
  screen pinch / ctrl+scroll zoom
  stylus button = last eraser (hold)   air-click = pen ↔ eraser
  2nd button = lasso (hold)
  drag the pencil-case handle → top / bottom / sides
  hold the pen still ~1s ≈ line, arrow, triangle, square, diamond, ellipse, circle
  ctrl+z/y   ctrl+e png   ctrl+shift+e pdf
";

fn main() -> eframe::Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--help") | Some("-h") => {
            print!("{HELP}");
            return Ok(());
        }
        Some("--version") | Some("-V") => {
            println!("cahier {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("--data-dir") => {
            println!("{}", crate::library::data_dir().display());
            return Ok(());
        }
        Some("--theme") => {
            let look = crate::look::Look::load();
            println!("theme: {}", look.name.to_lowercase());
            println!(
                "file:  {}",
                crate::look::current_dir().join("colors.toml").display()
            );
            println!("desk:  {}", hex(look.desk));
            println!("accent:{}", hex(look.accent));
            println!("ink:   {}", hex(look.fg));
            if let Some(font) = crate::fonts::mono_file() {
                println!("mono:  {}", font.display());
            }
            return Ok(());
        }
        Some(other) => {
            eprintln!("Error: unknown argument `{other}`.");
            eprintln!("  cahier --help");
            std::process::exit(2);
        }
        None => {}
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([800.0, 560.0])
            .with_title("Notes")
            .with_app_id("com.clouddown.cahier"),
        vsync: false,
        ..Default::default()
    };
    eframe::run_native(
        "Notes",
        options,
        Box::new(|cc| Ok(Box::new(CahierApp::new(cc)))),
    )
}

fn hex(c: egui::Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}
