//! Réglages et file d'attente persistés (JSON dans le dossier de données de l'appli).

use dlcore::APP_NAME;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Lang {
    En,
    Fr,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub connections: usize,
    pub folder: String,
    pub lang: Lang,
    /// Limite globale par téléchargement en Mo/s (0 = aucune).
    pub speed_limit_mb: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            connections: 8,
            folder: default_folder(),
            lang: system_lang(),
            speed_limit_mb: 0.0,
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
}

fn data_dir() -> PathBuf {
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
    let d = data_dir();
    let _ = std::fs::create_dir_all(&d);
    if let Ok(json) = serde_json::to_vec_pretty(v) {
        let _ = dlcore::engine::write_atomic(&d.join(name), &json);
    }
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
