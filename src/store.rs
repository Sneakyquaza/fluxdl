//! Réglages et file d'attente persistés (JSON dans le dossier de données de l'appli).

use dlcore::APP_NAME;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Lang {
    En,
    Fr,
}

impl Lang {
    /// Code de langue (sous-titres dans la langue de l'interface).
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Fr => "fr",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub connections: usize,
    pub folder: String,
    pub lang: Lang,
    /// Limite globale en Mo/s, partagée par tous les téléchargements (0 = aucune).
    pub speed_limit_mb: f32,
    /// Serveur local pour l'extension navigateur : actif, port, jeton d'appairage.
    pub server: bool,
    pub port: u16,
    pub token: String,
    /// Torrents : partage après la fin jusqu'à ce ratio (0 = arrêt dès la fin).
    pub seed_ratio: f32,
    /// Windows : liens magnet et .torrent ouverts par l'appli ; question déjà posée.
    pub torrent_handler: bool,
    pub handler_asked: bool,
    /// Dernière mise à jour de yt-dlp (secondes Unix).
    pub ytdlp_updated: u64,
    /// Lien téléchargeable copié ailleurs : le proposer dans une bulle.
    pub clipboard_watch: bool,
    /// Bulle « Installer l'extension » déjà montrée (ou guide déjà ouvert).
    pub extension_asked: bool,
    /// Vidéo envoyée par l'extension fenêtre réduite : ajoutée sans question dans cette qualité
    /// (fenêtre visible : choix du format, cette qualité cochée d'avance).
    pub video_quality: crate::video::Qualite,
    /// Sous-titres, miniature, chapitres, conteneur : derniers choix de la fenêtre des formats
    /// (langue des sous-titres non gardée : celle de l'interface si la vidéo l'a).
    pub video_options: crate::video::Sortie,
    /// Planification : la file démarre à cette heure, se met en pause à celle-là (minutes depuis
    /// minuit, heure locale ; `None` = aucune).
    pub plan_start: Option<u16>,
    pub plan_stop: Option<u16>,
    /// Quand tout est fini. Veille et arrêt du PC ne sont pas gardés d'une session à l'autre
    /// (remis à `Rien` au lancement).
    pub end_action: Fin,
}

/// Action quand tous les téléchargements sont finis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Fin {
    #[default]
    Rien,
    /// Fermer fluxdl.
    Fermer,
    /// Mettre le PC en veille, éteindre le PC (compte à rebours de 60 s, annulable).
    Veille,
    Eteindre,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            connections: 8,
            folder: default_folder(),
            lang: system_lang(),
            speed_limit_mb: 0.0,
            server: true,
            port: crate::serveur::PORT,
            token: String::new(),
            seed_ratio: 0.0,
            torrent_handler: false,
            handler_asked: false,
            ytdlp_updated: 0,
            clipboard_watch: true,
            extension_asked: false,
            video_quality: crate::video::Qualite::Best,
            // Miniature et chapitres intégrés (sans ffmpeg, ignorés au téléchargement).
            video_options: crate::video::Sortie {
                thumbnail: true,
                chapters: true,
                ..Default::default()
            },
            plan_start: None,
            plan_stop: None,
            end_action: Fin::Rien,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Status {
    Queued,
    Running,
    Paused,
    Done,
    Failed(String),
    Canceled,
}

/// Sorte de téléchargement : fichier HTTP (moteur multi-connexions), vidéo (yt-dlp), torrent.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Kind {
    #[default]
    Http,
    /// `section` : passage (secondes), `from_start` : live depuis son début, `login` : lancé avec
    /// les cookies de connexion du site (gardés en mémoire seulement, jamais ici).
    Video {
        quality: crate::video::Qualite,
        estimate: Option<u64>,
        #[serde(default)]
        section: Option<(u32, u32)>,
        #[serde(default)]
        from_start: bool,
        #[serde(default)]
        login: bool,
        /// Sous-titres, miniature, chapitres, conteneur (absent avant 0.2.9 : rien d'ajouté).
        #[serde(default)]
        options: crate::video::Sortie,
    },
    /// `.torrent` gardé sous `torrents/<hash>.torrent` ; `started` : fichiers déjà à nous
    /// (reprise permise), `files` : fichiers choisis (tous si aucun).
    Torrent {
        hash: String,
        files: Option<Vec<usize>>,
        started: bool,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: u64,
    pub url: String,
    pub dir: String,
    /// Nom choisi à la sonde : gardé pour reprendre le même `.part`.
    pub name: Option<String>,
    pub sha256: Option<String>,
    pub connections: usize,
    pub status: Status,
    pub size: Option<u64>,
    pub done: u64,
    #[serde(default)]
    pub kind: Kind,
    /// Venus de l'extension : page d'origine et cookies (sites à connexion), renvoyés au serveur.
    #[serde(default)]
    pub referrer: Option<String>,
    #[serde(default)]
    pub cookies: Option<String>,
    /// Limite propre à ce téléchargement en Mo/s (0 = aucune), en plus de la globale.
    #[serde(default)]
    pub speed_limit_mb: f32,
}

/// Erreur d'un téléchargement commencé dont le `.part` a disparu (effacé à la main).
pub const ERR_PARTIEL: &str = "Partial file not found.";

/// Au lancement : un fichier HTTP commencé (nom choisi, `.part` réservé) ne reprend tout seul que
/// si son `.part` est encore là ; sinon « fichier partiel introuvable » (Recommencer / Retirer) :
/// jamais un gros fichier recréé sans l'utilisateur.
pub fn au_lancement(e: &mut Entry) {
    let commence = e.kind == Kind::Http
        && matches!(e.status, Status::Running | Status::Queued | Status::Paused);
    if let Some(n) = e.name.as_ref().filter(|_| commence) {
        let cible = std::path::Path::new(&e.dir).join(n);
        if !dlcore::engine::part_path(&cible).exists() {
            e.status = Status::Failed(ERR_PARTIEL.into());
            return;
        }
    }
    // Interrompu par un arrêt ou un plantage : reprend tout seul.
    if e.status == Status::Running {
        e.status = Status::Queued;
    }
}

pub fn data_dir() -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.unwrap_or_else(std::env::temp_dir).join(APP_NAME)
}

pub fn load<T: DeserializeOwned>(name: &str) -> Option<T> {
    serde_json::from_slice(&std::fs::read(data_dir().join(name)).ok()?).ok()
}

pub fn save<T: Serialize>(name: &str, v: &T) {
    save_in(&data_dir(), name, v);
}

fn save_in<T: Serialize>(d: &std::path::Path, name: &str, v: &T) {
    let _ = std::fs::create_dir_all(d);
    // Jeton et cookies dedans : dossier et fichiers lisibles par l'utilisateur seul.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700));
    }
    if let Ok(json) = serde_json::to_vec_pretty(v) {
        let f = d.join(name);
        let _ = dlcore::engine::write_atomic(&f, &json);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600));
        }
    }
}

/// File d'attente lue entrée par entrée : une entrée illisible (version plus récente, fichier
/// abîmé à la main) ne vide pas la file. Elle est gardée telle quelle (`inconnues`, réécrite à
/// chaque sauvegarde) et le fichier d'origine est copié en `<nom>.bak`. Fichier entièrement
/// illisible : copié en `.bak`, file vide.
pub fn load_queue(d: &std::path::Path, name: &str) -> (Vec<Entry>, Vec<serde_json::Value>) {
    let f = d.join(name);
    let Ok(octets) = std::fs::read(&f) else {
        return Default::default();
    };
    let bak = || {
        let _ = std::fs::copy(&f, d.join(format!("{name}.bak")));
    };
    let Ok(valeurs) = serde_json::from_slice::<Vec<serde_json::Value>>(&octets) else {
        bak();
        return Default::default();
    };
    let (mut bonnes, mut inconnues) = (Vec::new(), Vec::new());
    for v in valeurs {
        match serde_json::from_value::<Entry>(v.clone()) {
            Ok(e) => bonnes.push(e),
            Err(_) => inconnues.push(v),
        }
    }
    if !inconnues.is_empty() {
        bak();
    }
    (bonnes, inconnues)
}

/// Écrit la file : entrées connues puis inconnues gardées telles quelles.
pub fn save_queue(
    d: &std::path::Path,
    name: &str,
    entries: &[Entry],
    inconnues: &[serde_json::Value],
) {
    let mut v: Vec<serde_json::Value> = entries
        .iter()
        .filter_map(|e| serde_json::to_value(e).ok())
        .collect();
    v.extend(inconnues.iter().cloned());
    save_in(d, name, &v);
}

fn default_folder() -> String {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    home.map(|h| PathBuf::from(h).join("Downloads"))
        .unwrap_or_else(std::env::temp_dir)
        .display()
        .to_string()
}

/// Français si le système l'est, anglais sinon.
fn system_lang() -> Lang {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetUserDefaultUILanguage() -> u16;
        }
        // Les 10 bits bas = langue principale (0x0C = français).
        if unsafe { GetUserDefaultUILanguage() } & 0x3ff == 0x0c {
            return Lang::Fr;
        }
    }
    #[cfg(not(windows))]
    for v in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(l) = std::env::var(v) {
            if !l.is_empty() {
                return if l.starts_with("fr") {
                    Lang::Fr
                } else {
                    Lang::En
                };
            }
        }
    }
    Lang::En
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Audit M1 : une entrée d'un `Kind` inconnu (version plus récente) ne vide plus la file.
    #[test]
    fn file_tolerante_entree_par_entree() {
        let d = std::env::temp_dir().join(format!("{APP_NAME}-file-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let bonne = r#"{"id":1,"url":"https://h/a.zip","dir":"/t","name":null,"sha256":null,
            "connections":8,"status":"Queued","size":null,"done":0}"#;
        let futur = r#"{"id":2,"url":"https://h/b","dir":"/t","name":null,"sha256":null,
            "connections":8,"status":"Queued","size":null,"done":0,"kind":{"Ipfs":{"cid":"x"}}}"#;
        std::fs::write(d.join("q.json"), format!("[{bonne},{futur},{bonne}]")).unwrap();
        let (e, inconnues) = load_queue(&d, "q.json");
        assert_eq!((e.len(), inconnues.len()), (2, 1));
        assert!(d.join("q.json.bak").exists());
        // Réécrite : l'entrée inconnue est gardée.
        save_queue(&d, "q.json", &e, &inconnues);
        let (e2, i2) = load_queue(&d, "q.json");
        assert_eq!((e2.len(), i2.len()), (2, 1));
        assert_eq!(i2[0]["kind"]["Ipfs"]["cid"], "x");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = std::fs::metadata(d.join("q.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(m & 0o777, 0o600);
        }
        // Fichier illisible : copié en .bak, file vide (jamais d'écrasement sans copie).
        std::fs::write(d.join("c.json"), b"[{\"id\":").unwrap();
        assert_eq!(load_queue(&d, "c.json").0.len(), 0);
        assert_eq!(std::fs::read(d.join("c.json.bak")).unwrap(), b"[{\"id\":");
    }

    #[test]
    fn partiel_efface_pas_recree() {
        let d = std::env::temp_dir().join(format!("{APP_NAME}-lancement-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let entree = |name: Option<&str>, status| Entry {
            id: 1,
            url: "https://h/f.rar".into(),
            dir: d.display().to_string(),
            name: name.map(str::to_string),
            sha256: None,
            connections: 8,
            status,
            size: Some(62 << 30),
            done: 1 << 30,
            kind: Kind::Http,
            referrer: None,
            cookies: None,
            speed_limit_mb: 0.0,
        };
        // .part effacé à la main : ni reprise ni fichier recréé.
        for st in [Status::Running, Status::Queued, Status::Paused] {
            let mut e = entree(Some("gros.rar"), st);
            au_lancement(&mut e);
            assert_eq!(e.status, Status::Failed(ERR_PARTIEL.into()));
        }
        assert!(!d.join("gros.rar.part").exists());
        // .part présent : reprend.
        std::fs::write(d.join("la.rar.part"), b"").unwrap();
        let mut e = entree(Some("la.rar"), Status::Running);
        au_lancement(&mut e);
        assert_eq!(e.status, Status::Queued);
        // Jamais commencé (pas de nom) : part normalement.
        let mut e = entree(None, Status::Queued);
        au_lancement(&mut e);
        assert_eq!(e.status, Status::Queued);
        let _ = std::fs::remove_dir_all(&d);
    }
}
