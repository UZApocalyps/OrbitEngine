//! Command line of `orbit-editor`.

use std::path::PathBuf;

use orbit_editor_api::{Lang, ThemeMode};

pub const USAGE: &str = "\
orbit-editor [projet | histoire.orbit] [options]

Options :
  --config-dir <dossier>   dossier de configuration utilisateur (disposition, projets récents)
  --no-config              ne lit ni n'écrit aucune configuration
  --theme dark|light       force le thème
  --lang en|fr             force la langue de l'interface (sans l'enregistrer), pour les captures
  --size <LxH>             taille initiale de la fenêtre, en pixels logiques (ex. 1360x820)
  --autosave-secs <n>      délai de la sauvegarde automatique (défaut 30)
  --frame-counter          journalise chaque frame dessinée et affiche le compteur
  --select-node <n>        sélectionne le n-ième nœud du projet ouvert (0 = premier), pour les captures
  --run-action <id>        exécute une action au démarrage (répétable), ex. orbit.explorer.add_say
  --open-palette           ouvre la palette de commandes au démarrage
  --palette-query <texte>  texte saisi dans la palette (avec --open-palette)
  --search <texte>         ouvre la recherche globale avec ce texte
  --select-asset <n>       sélectionne la n-ième ressource du manifeste (0 = première), pour les captures
  --import <fichier>       importe un fichier (ou un dossier) dans le projet ouvert au démarrage,
                           comme un glisser-déposer (répétable)
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
  --generate-story <n> <fichier.orbit>
                           écrit une histoire de test d'environ <n> nœuds (un scène) puis quitte
  --generate-graph <n> <fichier.orbit>
                           écrit une histoire de test d'environ <n> blocs de graphe (scènes liées) puis quitte
  --autoscroll <px>        fait défiler l'éditeur de scène de <px> pixels par frame ; avec --exit-after,
                           journalise le temps de frame (moyenne, p95, max) à la sortie
  --mcp-port <port>        démarre le serveur MCP live (HTTP, 127.0.0.1) sur ce port au lieu d'attendre
                           le bouton du panneau « Assistant IA » (debug, scripts)
  --mcp-token <jeton>      jeton d'accès à utiliser avec --mcp-port (sinon celui des préférences)
  -h, --help               affiche cette aide
";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Args {
    pub project: Option<PathBuf>,
    pub config_dir: Option<PathBuf>,
    pub no_config: bool,
    pub theme: Option<ThemeMode>,
    pub lang: Option<Lang>,
    pub size: Option<(f64, f64)>,
    pub autosave_secs: Option<u64>,
    pub frame_counter: bool,
    pub select_node: Option<usize>,
    pub run_actions: Vec<String>,
    pub open_palette: bool,
    pub palette_query: String,
    pub search: Option<String>,
    pub select_asset: Option<usize>,
    pub imports: Vec<PathBuf>,
    pub screenshot: Option<PathBuf>,
    pub exit_after: Option<f32>,
    pub play_preview: bool,
    pub always_active: bool,
    pub preview_skip: bool,
    pub play_from_selection: bool,
    pub edit_text: Option<(usize, String)>,
    pub edit_delay: f32,
    pub preview_capture: Option<PathBuf>,
    pub generate_story: Option<(usize, PathBuf)>,
    pub generate_graph: Option<(usize, PathBuf)>,
    pub autoscroll: Option<f32>,
    pub mcp_port: Option<u16>,
    pub mcp_token: Option<String>,
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
                "--generate-story" => {
                    let n = value("--generate-story")?;
                    let n = n.parse().map_err(|_| format!("nombre invalide : {n}"))?;
                    let path = PathBuf::from(value("--generate-story")?);
                    out.generate_story = Some((n, path));
                }
                "--generate-graph" => {
                    let n = value("--generate-graph")?;
                    let n = n.parse().map_err(|_| format!("nombre invalide : {n}"))?;
                    let path = PathBuf::from(value("--generate-graph")?);
                    out.generate_graph = Some((n, path));
                }
                "--mcp-port" => {
                    let v = value("--mcp-port")?;
                    out.mcp_port = Some(v.parse().map_err(|_| format!("port invalide : {v}"))?);
                }
                "--mcp-token" => out.mcp_token = Some(value("--mcp-token")?),
                "--autoscroll" => {
                    let v = value("--autoscroll")?;
                    out.autoscroll = Some(v.parse().map_err(|_| format!("nombre invalide : {v}"))?);
                }
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
                "--search" => out.search = Some(value("--search")?),
                "--select-asset" => {
                    let v = value("--select-asset")?;
                    out.select_asset =
                        Some(v.parse().map_err(|_| format!("nombre invalide : {v}"))?);
                }
                "--import" => out.imports.push(PathBuf::from(value("--import")?)),
                "--screenshot" => out.screenshot = Some(PathBuf::from(value("--screenshot")?)),
                "--theme" => {
                    out.theme = Some(match value("--theme")?.as_str() {
                        "dark" | "sombre" => ThemeMode::Dark,
                        "light" | "clair" => ThemeMode::Light,
                        other => return Err(format!("thème inconnu : {other}")),
                    });
                }
                "--lang" => {
                    let v = value("--lang")?;
                    out.lang =
                        Some(Lang::from_tag(&v).ok_or_else(|| format!("langue inconnue : {v}"))?);
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
        assert_eq!(parse(&["--lang", "fr"]).unwrap().lang, Some(Lang::Fr));
        assert_eq!(parse(&["--lang", "en-US"]).unwrap().lang, Some(Lang::En));
        assert_eq!(args.size, Some((1000.0, 700.0)));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse(&["--nope"]).is_err());
        assert!(parse(&["--theme", "pink"]).is_err());
        assert!(parse(&["--lang", "de"]).is_err());
        assert!(parse(&["--size", "wide"]).is_err());
        assert!(parse(&["--exit-after"]).is_err());
        assert!(parse(&["a", "b"]).is_err());
    }
}
