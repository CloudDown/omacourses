mod app;
mod camera;
mod document;
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
cahier — notes au stylet, pupitre Omarchy

Usage:
  cahier
  cahier --help
  cahier --version
  cahier --data-dir
  cahier --theme

Ouvre le pupitre. Les cahiers vivent dans $CAHIER_DATA
ou ~/.local/share/omacourses. Le chrome suit
~/.local/state/omarchy/current/theme (même palette que le terminal).

Examples:
  cahier
  CAHIER_DATA=/tmp/cahier cahier
  CAHIER_OPEN=Mentalité cahier
  cahier --theme
  cahier --data-dir

Gestes (dans un cahier):
  p feutre   b plume   c crayon   h surligneur
  e gomme (re-clic = zone)   l lasso   t texte
  [ ] épaisseur   1-9 encre   m papier
  + − zoom   0 / coins = taille écran
  pavé tactile : pincer ou deux doigts (vertical) = zoom
  pincement écran / ctrl+molette zoom
  espace+glisser panorama
  glisser la poignée de la trousse → haut / bas / côtés
  shift relâché après un trait ≈ forme (ligne, cercle, rectangle)
  clic droit = gomme   ctrl+z/y   ctrl+e png   ctrl+shift+e pdf
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
            println!("file:  {}", crate::look::current_dir().join("colors.toml").display());
            println!("desk:  {}", hex(look.desk));
            println!("accent:{}", hex(look.accent));
            println!("ink:   {}", hex(look.fg));
            if let Some(font) = crate::fonts::mono_file() {
                println!("mono:  {}", font.display());
            }
            return Ok(());
        }
        Some(other) => {
            eprintln!("Error: argument inconnu `{other}`.");
            eprintln!("  cahier --help");
            std::process::exit(2);
        }
        None => {}
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([800.0, 560.0])
            .with_title("Cahier")
            .with_app_id("com.clouddown.cahier"),
        vsync: true,
        ..Default::default()
    };
    eframe::run_native(
        "Cahier",
        options,
        Box::new(|cc| Ok(Box::new(CahierApp::new(cc)))),
    )
}

fn hex(c: egui::Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}
