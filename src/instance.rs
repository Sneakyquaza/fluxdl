//! Instance unique : verrou sur `<données>/instance.lock`, tenu tant que l'appli tourne
//! (Windows : fichier ouvert sans partage ; Linux : `flock`). Le système le relâche à la fin du
//! processus, même après un plantage. Deux instances n'écrivent donc jamais le même `.part` ni
//! `queue.json` : la 2e confie ses liens à la 1re (serveur local) et s'arrête.

use std::fs::File;
use std::path::Path;
use std::time::{Duration, Instant};

/// Verrou pris (à garder en vie jusqu'à la fin).
pub struct Verrou(#[allow(dead_code)] File);

/// Essaie de prendre le verrou ; `None` : une autre instance le tient.
pub fn prendre(dossier: &Path) -> Option<Verrou> {
    std::fs::create_dir_all(dossier).ok()?;
    let chemin = dossier.join("instance.lock");
    let mut o = std::fs::OpenOptions::new();
    o.create(true).truncate(false).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Aucun partage : une 2e ouverture échoue (violation de partage) tant que la 1re vit.
        o.share_mode(0);
    }
    let f = o.open(chemin).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        // SAFETY : descripteur valide, gardé ouvert par `f`.
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return None;
        }
    }
    Some(Verrou(f))
}

/// Comme `prendre`, en réessayant pendant `delai` (relance après une mise à jour : l'ancienne
/// version finit de se fermer).
pub fn attendre(dossier: &Path, delai: Duration) -> Option<Verrou> {
    let fin = Instant::now() + delai;
    loop {
        if let Some(v) = prendre(dossier) {
            return Some(v);
        }
        if Instant::now() >= fin {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_seule_instance() {
        let d = std::env::temp_dir().join(format!("fluxdl-verrou-{}", std::process::id()));
        let v = prendre(&d).expect("1re instance");
        assert!(prendre(&d).is_none(), "2e instance refusée");
        assert!(attendre(&d, Duration::from_millis(150)).is_none());
        drop(v);
        assert!(prendre(&d).is_some(), "verrou rendu à la fin");
    }
}
