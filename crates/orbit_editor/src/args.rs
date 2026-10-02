//! Command line of `orbit-editor`.

use std::path::PathBuf;

use orbit_editor_api::ThemeMode;

pub const USAGE: &str = "\
orbit-editor [projet | histoire.ron] [options]

Options :
  --config-dir <dossier>   dossier de configuration utilisateur (disposition, projets récents)
  --no-config              ne lit ni n'écrit aucune configuration
  --theme dark|light       force le thème
  --size <LxH>             taille initiale de la fenêtre, en pixels logiques (ex. 1360x820)
  --autosave-secs <n>      délai de la sauvegarde automatique (défaut 30)
  --frame-counter          journalise chaque frame dessinée et affiche le compteur
  --select-node <n>        sélectionne le n-ième nœud du projet ouvert (0 = premier), pour les captures
  --run-action <id>        exécute une action au démarrage (répétable), ex. orbit.explorer.add_say
  --open-palette           ouvre la palette de commandes au démarrage
  --palette-query <texte>  texte saisi dans la palette (avec --open-palette)
  --play-preview           lance la preview (depuis le début) au démarrage
  --play-from-selection    lance la preview « depuis ici » sur le nœud de --select-node
  --edit-text <n>=<texte>  modifie le texte de la n-ième réplique (0 = première) après --edit-delay,
                           comme une frappe : la preview doit se recharger (mesure de latence)
  --edit-delay <secondes>  délai avant --edit-text (défaut 1.5)
  --preview-skip           avec --play-preview : active « passer » (s'arrête au premier choix)
  --always-active          la preview continue quand la fenêtre perd le focus (captures automatisées)
  --preview-capture <png>  enregistre l'image de la preview (relecture de sa cible, 16:9) puis quitte
  --screenshot <png>       enregistre la frame rendue dans un PNG puis quitte
  --exit-after <secondes>  quitte après ce délai (la capture est prise à ce moment) ; comme un
                           arrêt brutal : pas de confirmation, la sauvegarde automatique reste
  -h, --help               affiche cette aide
";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Args {
    pub project: Option<PathBuf>,
    pub config_dir: Option<PathBuf>,
    pub no_config: bool,
    pub theme: Option<ThemeMode>,
    pub size: Option<(f64, f64)>,
    pub autosave_secs: Option<u64>,
    pub frame_counter: bool,
    pub select_node: Option<usize>,
    pub run_actions: Vec<String>,
    pub open_palette: bool,
    pub palette_query: String,
    pub screenshot: Option<PathBuf>,
    pub exit_after: Option<f32>,
    pub play_preview: bool,
    pub always_active: bool,
    pub preview_skip: bool,
    pub play_from_selection: bool,
    pub edit_text: Option<(usize, String)>,
    pub edit_delay: f32,
    pub preview_capture: Option<PathBuf>,
    pub help: bool,
}

impl Args {
    pub fn parse<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut out = Args {
            edit_delay: 1.5,
            ..Args::default()
        };
        let mut it = args.into_iter();
        while let Some(arg) = it.next() {
            let mut value =
                |name: &str| it.next().ok_or_else(|| format!("{name} attend une valeur"));
            match arg.as_str() {
                "-h" | "--help" => out.help = true,
                "--no-config" => out.no_config = true,
                "--frame-counter" => out.frame_counter = true,
                "--open-palette" => out.open_palette = true,
                "--play-preview" => out.play_preview = true,
                "--always-active" => out.always_active = true,
                "--preview-skip" => out.preview_skip = true,
                "--play-from-selection" => out.play_from_selection = true,
                "--preview-capture" => {
                    out.preview_capture = Some(PathBuf::from(value("--preview-capture")?));
                }
                "--edit-text" => {
                    let v = value("--edit-text")?;
                    let (n, text) = v.split_once('=').ok_or("--edit-text attend <n>=<texte>")?;
                    let n = n.parse().map_err(|_| format!("nombre invalide : {n}"))?;
                    out.edit_text = Some((n, text.to_owned()));
                }
                "--edit-delay" => {
                    let v = value("--edit-delay")?;
                    out.edit_delay = v.parse().map_err(|_| format!("nombre invalide : {v}"))?;
                }
                "--config-dir" => out.config_dir = Some(PathBuf::from(value("--config-dir")?)),
                "--palette-query" => out.palette_query = value("--palette-query")?,
                "--screenshot" => out.screenshot = Some(PathBuf::from(value("--screenshot")?)),
                "--theme" => {
                    out.theme = Some(match value("--theme")?.as_str() {
                        "dark" | "sombre" => ThemeMode::Dark,
                        "light" | "clair" => ThemeMode::Light,
                        other => return Err(format!("thème inconnu : {other}")),
                    });
                }
                "--size" => {
                    let v = value("--size")?;
                    let (w, h) = v
                        .split_once(['x', 'X'])
                        .ok_or("--size attend LxH (ex. 1360x820)")?;
                    let parse = |s: &str| {
                        s.parse::<f64>()
                            .map_err(|_| format!("taille invalide : {v}"))
                    };
                    out.size = Some((parse(w)?, parse(h)?));
                }
                "--autosave-secs" => {
                    let v = value("--autosave-secs")?;
                    out.autosave_secs =
                        Some(v.parse().map_err(|_| format!("nombre invalide : {v}"))?);
                }
                "--run-action" => out.run_actions.push(value("--run-action")?),
                "--select-node" => {
                    let v = value("--select-node")?;
                    out.select_node =
                        Some(v.parse().map_err(|_| format!("nombre invalide : {v}"))?);
                }
                "--exit-after" => {
                    let v = value("--exit-after")?;
                    out.exit_after = Some(v.parse().map_err(|_| format!("nombre invalide : {v}"))?);
                }
                flag if flag.starts_with('-') => return Err(format!("option inconnue : {flag}")),
                path if out.project.is_none() => out.project = Some(PathBuf::from(path)),
                _ => return Err("un seul projet peut être ouvert au démarrage".to_owned()),
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        Args::parse(args.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn parses_the_debug_options() {
        let args = parse(&[
            "demo/",
            "--screenshot",
            "out.png",
            "--exit-after",
            "2.5",
            "--open-palette",
            "--theme",
            "light",
            "--size",
            "1000x700",
            "--frame-counter",
        ])
        .unwrap();
        assert_eq!(args.project, Some(PathBuf::from("demo/")));
        assert_eq!(args.screenshot, Some(PathBuf::from("out.png")));
        assert_eq!(args.exit_after, Some(2.5));
        assert!(args.open_palette && args.frame_counter);
        assert_eq!(args.theme, Some(ThemeMode::Light));
        assert_eq!(args.size, Some((1000.0, 700.0)));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse(&["--nope"]).is_err());
        assert!(parse(&["--theme", "pink"]).is_err());
        assert!(parse(&["--size", "wide"]).is_err());
        assert!(parse(&["--exit-after"]).is_err());
        assert!(parse(&["a", "b"]).is_err());
    }
}
