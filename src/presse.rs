//! Surveillance du presse-papiers : un fil lit le texte toutes les 700 ms et ne réveille l'UI
//! que si un lien téléchargeable vient d'y être copié (0 image au repos).

use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Duration;

/// Extensions de fichiers proposées au téléchargement quand leur lien est copié.
const EXTENSIONS: &[&str] = &[
    "zip",
    "7z",
    "rar",
    "tar",
    "gz",
    "tgz",
    "xz",
    "bz2",
    "zst",
    "iso",
    "img",
    "dmg",
    "exe",
    "msi",
    "msix",
    "apk",
    "deb",
    "rpm",
    "appimage",
    "bin",
    "pdf",
    "epub",
    "mp4",
    "mkv",
    "webm",
    "avi",
    "mov",
    "mp3",
    "flac",
    "wav",
    "ogg",
    "m4a",
    "torrent",
    "safetensors",
    "gguf",
    "onnx",
    "ckpt",
];

/// Lien seul et téléchargeable : magnet, http(s) vers une extension connue, ou page d'un site
/// vidéo (yt-dlp). Sinon `None`.
pub fn lien(texte: &str) -> Option<String> {
    let s = texte.trim();
    if s.is_empty() || s.len() > 8192 || s.chars().any(char::is_whitespace) {
        return None;
    }
    if s.starts_with("magnet:?") {
        return Some(s.into());
    }
    let bas = s.to_ascii_lowercase();
    if !(bas.starts_with("http://") || bas.starts_with("https://")) {
        return None;
    }
    let nom = dlcore::engine::name_from_url(s)?;
    if crate::video::site_video(s) {
        return Some(s.into());
    }
    let ext = nom.rsplit_once('.')?.1.to_ascii_lowercase();
    EXTENSIONS.contains(&ext.as_str()).then(|| s.into())
}

/// Suit les changements du texte copié ; ne propose pas deux fois de suite le même lien.
#[derive(Default)]
pub struct Veille {
    /// Dernier texte lu (`None` : pas encore lu, le contenu du lancement n'est pas proposé).
    vu: Option<String>,
    propose: Option<String>,
}

impl Veille {
    pub fn suivant(&mut self, texte: &str) -> Option<String> {
        if self.vu.as_deref() == Some(texte) {
            return None;
        }
        let premier = self.vu.is_none();
        self.vu = Some(texte.to_string());
        let l = lien(texte).filter(|l| !premier && self.propose.as_ref() != Some(l))?;
        self.propose = Some(l.clone());
        Some(l)
    }
}

/// Numéro de version du presse-papiers (Windows), qui change à chaque copie ; ailleurs `None`.
#[cfg(windows)]
fn sequence() -> Option<u32> {
    #[link(name = "user32")]
    extern "system" {
        fn GetClipboardSequenceNumber() -> u32;
    }
    // SAFETY : sans argument, lecture d'un compteur du système.
    Some(unsafe { GetClipboardSequenceNumber() }).filter(|&n| n != 0)
}

#[cfg(not(windows))]
fn sequence() -> Option<u32> {
    None
}

/// Lance le fil de surveillance ; `actif` faux = aucune lecture. Les liens arrivent par le
/// canal, l'UI est réveillée à chacun.
pub fn lancer(ctx: egui::Context, actif: Arc<AtomicBool>) -> Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut veille = Veille::default();
        let mut presse = None;
        let mut numero = None;
        loop {
            std::thread::sleep(Duration::from_millis(700));
            if !actif.load(Ordering::Relaxed) {
                // Réactivé plus tard : le contenu d'alors n'est pas proposé.
                veille.vu = None;
                continue;
            }
            // Windows : lecture seulement après un changement (ouvrir le presse-papiers toutes
            // les 700 ms peut le rendre « occupé » pour les autres applis).
            let n = sequence();
            if n.is_some() && n == numero && veille.vu.is_some() {
                continue;
            }
            numero = n;
            if presse.is_none() {
                presse = arboard::Clipboard::new().ok();
            }
            let Some(c) = presse.as_mut() else {
                continue;
            };
            // Presse-papiers vide ou non textuel : comme un texte vide.
            let texte = c.get_text().unwrap_or_default();
            if let Some(l) = veille.suivant(&texte) {
                if tx.send(l).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liens_reconnus() {
        let ok = |s: &str| lien(s).is_some();
        assert!(ok("https://example.com/files/ubuntu.iso"));
        assert!(ok("  https://a.b/x/archive.TAR.GZ?token=1 \n"));
        assert!(ok("magnet:?xt=urn:btih:abc"));
        assert!(ok("https://www.youtube.com/watch?v=aqz-KE-bpKQ"));
        assert!(!ok("https://www.youtube.com/"));
        assert!(!ok("https://example.com/page"));
        assert!(!ok("https://example.com/index.html"));
        assert!(!ok("ftp://a.b/f.zip"));
        assert!(!ok("voir https://a.b/f.zip"));
        assert!(!ok("https://a.b/f.zip https://a.b/g.zip"));
        assert!(!ok(""));
    }

    #[test]
    fn pas_deux_fois_de_suite() {
        let mut v = Veille::default();
        // Contenu présent au lancement : pas proposé.
        assert_eq!(v.suivant("https://a.b/f.zip"), None);
        assert_eq!(v.suivant("bonjour"), None);
        assert_eq!(
            v.suivant("https://a.b/g.zip").as_deref(),
            Some("https://a.b/g.zip")
        );
        assert_eq!(v.suivant("https://a.b/g.zip"), None);
        // Recopié après autre chose : même lien, pas reproposé.
        assert_eq!(v.suivant("texte"), None);
        assert_eq!(v.suivant("https://a.b/g.zip"), None);
        assert_eq!(
            v.suivant("https://a.b/f.zip").as_deref(),
            Some("https://a.b/f.zip")
        );
    }
}
