//! Mise à jour depuis les GitHub Releases de `Sneakyquaza/fluxdl` : dernière version lue sur
//! un autre fil (lancement puis toutes les 6 h), `fluxdl.exe` téléchargé et vérifié (sha256) à
//! côté de l'exécutable, fichiers échangés (Windows permet de renommer l'exe lancé), relance
//! après la fermeture propre. Jamais d'installation sans un clic. Linux : annonce seulement.

use crate::github::{self, suffixe, Erreur, Fichier};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const DEPOT: &str = "Sneakyquaza/fluxdl";
pub const PAGE_RELEASES: &str = "https://github.com/Sneakyquaza/fluxdl/releases/latest";
const NOM_EXE: &str = "fluxdl.exe";
/// Remplacement de l'exe lancé : Windows seulement (les releases ne publient que l'exe Windows).
pub const AUTO: bool = cfg!(windows);

/// Dernière version publiée.
#[derive(Clone, Debug, PartialEq)]
pub struct Annonce {
    pub version: String,
    pub exe: Fichier,
    pub sha256: Fichier,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Etat {
    Rien,
    Recherche,
    AJour,
    Erreur(Erreur),
    Dispo(Annonce),
    /// Version, octets reçus, total.
    Telechargement(String, u64, u64),
    /// Nouvel exécutable vérifié, prêt à remplacer l'actuel.
    Pret(String, PathBuf),
    /// Téléchargement ou installation échoués : rien n'a été changé.
    Echec(Erreur),
}

pub struct Maj {
    etat: Arc<Mutex<Etat>>,
    /// Exécutable lu au lancement (après un renommage, `current_exe` suivrait le `.ancien`).
    exe: Option<PathBuf>,
    reveil: Arc<dyn Fn() + Send + Sync>,
}

impl Maj {
    pub fn new(reveil: impl Fn() + Send + Sync + 'static) -> Maj {
        Maj {
            etat: Arc::new(Mutex::new(Etat::Rien)),
            exe: std::env::current_exe().ok(),
            reveil: Arc::new(reveil),
        }
    }

    pub fn etat(&self) -> Etat {
        self.etat.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn mettre(&self, e: Etat) {
        *self.etat.lock().unwrap_or_else(|e| e.into_inner()) = e;
    }

    /// Lit la dernière release sur un autre fil. `silencieux` (lancement, toutes les 6 h) :
    /// un échec ne laisse aucune trace, une annonce déjà là est gardée.
    pub fn chercher(&self, silencieux: bool) {
        {
            let mut e = self.etat.lock().unwrap_or_else(|e| e.into_inner());
            let occupe = matches!(
                *e,
                Etat::Recherche | Etat::Telechargement(..) | Etat::Pret(..)
            );
            if occupe || (silencieux && matches!(*e, Etat::Dispo(_))) {
                return;
            }
            *e = Etat::Recherche;
        }
        let (etat, r) = (Arc::clone(&self.etat), Arc::clone(&self.reveil));
        std::thread::spawn(move || {
            let res = match github::derniere(DEPOT).and_then(|rel| annonce(&rel)) {
                Ok(a) if github::plus_recente(&a.version, VERSION) => Etat::Dispo(a),
                Ok(_) => Etat::AJour,
                Err(_) if silencieux => Etat::Rien,
                Err(m) => Etat::Erreur(m),
            };
            *etat.lock().unwrap_or_else(|e| e.into_inner()) = res;
            r();
        });
    }

    /// Télécharge la version annoncée dans `<exe>.nouveau` (vérifiée), puis état `Pret`.
    pub fn telecharger(&self) {
        let Etat::Dispo(a) = self.etat() else { return };
        let Some(exe) = self.exe.clone() else {
            return self.mettre(Etat::Echec("Cannot find the app's location."));
        };
        self.mettre(Etat::Telechargement(a.version.clone(), 0, a.exe.size));
        let (etat, r) = (Arc::clone(&self.etat), Arc::clone(&self.reveil));
        std::thread::spawn(move || {
            let nouveau = suffixe(&exe, ".nouveau");
            let mut dernier = 0;
            let non = AtomicBool::new(false);
            let res = github::texte(&a.sha256.browser_download_url)
                .and_then(|t| github::empreinte(&t, NOM_EXE))
                .and_then(|sha| {
                    github::telecharger(&a.exe, &sha, &nouveau, &non, |faits| {
                        // Interface réveillée à chaque pour cent reçu, pas à chaque bloc.
                        if faits - dernier > a.exe.size / 100 {
                            dernier = faits;
                            *etat.lock().unwrap_or_else(|e| e.into_inner()) =
                                Etat::Telechargement(a.version.clone(), faits, a.exe.size);
                            r();
                        }
                    })
                });
            let fin = match res {
                Ok(()) => Etat::Pret(a.version.clone(), nouveau),
                Err(m) => Etat::Echec(m),
            };
            *etat.lock().unwrap_or_else(|e| e.into_inner()) = fin;
            r();
        });
    }

    /// Échange les fichiers (exe lancé → `.ancien`, `.nouveau` → exe lancé) et rend
    /// l'exécutable à relancer après la fermeture propre. Échec : rien n'a changé.
    pub fn installer(&self) -> Option<PathBuf> {
        let Etat::Pret(_, nouveau) = self.etat() else {
            return None;
        };
        let res = match &self.exe {
            Some(exe) => remplacer(exe, &nouveau).map(|()| exe.clone()).ok(),
            None => None,
        };
        if res.is_none() {
            let _ = std::fs::remove_file(&nouveau);
            self.mettre(Etat::Echec("App file locked: nothing was changed."));
        }
        res
    }
}

/// Release GitHub → annonce : `fluxdl.exe` et son `.sha256`, liens vers GitHub seulement.
pub fn annonce(r: &github::Release) -> Result<Annonce, Erreur> {
    let (Some(exe), Some(sha)) = (r.fichier(NOM_EXE), r.fichier(&format!("{NOM_EXE}.sha256")))
    else {
        return Err("The latest release has no Windows executable.");
    };
    if !github::url_permise(&exe.browser_download_url)
        || !github::url_permise(&sha.browser_download_url)
    {
        return Err("Download link refused (outside GitHub).");
    }
    Ok(Annonce {
        version: r.tag_name.trim_start_matches('v').to_string(),
        exe: exe.clone(),
        sha256: sha.clone(),
    })
}

/// Argument de la relance : la nouvelle version attend que l'ancienne rende le verrou.
pub const APRES_MAJ: &str = "--apres-maj";

/// Relance la nouvelle version (après la fermeture : file d'attente sauvée). Lancement
/// impossible : l'ancienne version reprend sa place.
pub fn relancer(exe: &Path) {
    if std::process::Command::new(exe)
        .arg(APRES_MAJ)
        .current_dir(crate::systeme::dossier_sur())
        .spawn()
        .is_err()
    {
        let nouveau = suffixe(exe, ".nouveau");
        if std::fs::rename(exe, &nouveau).is_ok() {
            let _ = std::fs::rename(suffixe(exe, ".ancien"), exe);
        }
        let _ = std::fs::remove_file(&nouveau);
    }
}

/// `exe` → `exe.ancien`, `nouveau` → `exe`. Échec du second renommage : l'exe d'origine est
/// remis.
fn remplacer(exe: &Path, nouveau: &Path) -> std::io::Result<()> {
    let ancien = suffixe(exe, ".ancien");
    let _ = std::fs::remove_file(&ancien);
    std::fs::rename(exe, &ancien)?;
    if let Err(e) = std::fs::rename(nouveau, exe) {
        let _ = std::fs::rename(&ancien, exe);
        return Err(e);
    }
    Ok(())
}

/// Lancement : supprime en fond l'ancienne version (et un téléchargement interrompu). Juste
/// après une mise à jour, l'ancien processus peut encore tenir son fichier : quelques essais.
pub fn nettoyer() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    std::thread::spawn(move || {
        let _ = std::fs::remove_file(suffixe(&exe, ".nouveau"));
        let ancien = suffixe(&exe, ".ancien");
        for _ in 0..60 {
            if !ancien.exists() || std::fs::remove_file(&ancien).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lecture_d_une_release() {
        let json = br#"{"tag_name":"v0.2.0","body":"Extension, videos, torrents.","assets":[
              {"name":"fluxdl.exe.sha256","size":80,"browser_download_url":
               "https://github.com/Sneakyquaza/fluxdl/releases/download/v0.2.0/fluxdl.exe.sha256"},
              {"name":"fluxdl-linux","size":11,"browser_download_url":
               "https://github.com/Sneakyquaza/fluxdl/releases/download/v0.2.0/fluxdl-linux"},
              {"name":"fluxdl.exe","size":12345678,"browser_download_url":
               "https://github.com/Sneakyquaza/fluxdl/releases/download/v0.2.0/fluxdl.exe"}]}"#;
        let r: github::Release = serde_json::from_slice(json).unwrap();
        let a = annonce(&r).unwrap();
        assert_eq!((a.version.as_str(), a.exe.size), ("0.2.0", 12_345_678));
        assert!(a.exe.browser_download_url.ends_with("/fluxdl.exe"));
        assert!(a
            .sha256
            .browser_download_url
            .ends_with("/fluxdl.exe.sha256"));
        // Sans l'exécutable ou sans son empreinte, ou un lien hors de GitHub : refusé.
        let sans: github::Release =
            serde_json::from_slice(br#"{"tag_name":"v0.2.0","body":null,"assets":[]}"#).unwrap();
        assert!(annonce(&sans).is_err());
        let sans_sha = String::from_utf8_lossy(json).replace("fluxdl.exe.sha256\"", "x\"");
        let r: github::Release = serde_json::from_str(&sans_sha).unwrap();
        assert!(annonce(&r).is_err());
        let pirate = String::from_utf8_lossy(json).replace(
            "https://github.com/Sneakyquaza/fluxdl/releases/download/v0.2.0/fluxdl.exe\"",
            "https://pirate.example/fluxdl.exe\"",
        );
        let r: github::Release = serde_json::from_str(&pirate).unwrap();
        assert!(annonce(&r).is_err());
    }

    #[test]
    fn remplacement_et_retour() {
        let d = std::env::temp_dir().join(format!("fluxdl-test-maj-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let exe = d.join("fluxdl.exe");
        let nouveau = suffixe(&exe, ".nouveau");
        std::fs::write(&exe, b"ancien").unwrap();
        std::fs::write(&nouveau, b"nouveau").unwrap();
        remplacer(&exe, &nouveau).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"nouveau");
        assert_eq!(std::fs::read(suffixe(&exe, ".ancien")).unwrap(), b"ancien");
        // Nouveau fichier absent : l'exe en place ne bouge pas.
        assert!(remplacer(&exe, &d.join("absent")).is_err());
        assert_eq!(std::fs::read(&exe).unwrap(), b"nouveau");
        let _ = std::fs::remove_dir_all(&d);
    }
}
