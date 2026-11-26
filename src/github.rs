//! Téléchargements depuis les GitHub Releases (mise à jour de l'appli, yt-dlp, ffmpeg) : liens
//! https vers GitHub seulement, chaque redirection vérifiée, taille et SHA-256 contrôlés avant
//! de garder le fichier. Modèle : `~/aquarium/src/maj.rs` (Deskquarium). Les erreurs sont des
//! clés anglaises traduites par l'UI (`i18n::t`).

use serde::Deserialize;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Hôtes autorisés (https seulement) : l'API, les pages des releases et leurs fichiers.
const HOTES: [&str; 4] = [
    "api.github.com",
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

pub type Erreur = &'static str;

#[derive(Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub assets: Vec<Fichier>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Fichier {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
}

impl Release {
    pub fn fichier(&self, nom: &str) -> Option<&Fichier> {
        self.assets.iter().find(|a| a.name == nom)
    }
}

/// https seulement, vers GitHub (API, pages des releases, fichiers des releases).
pub fn url_permise(url: &str) -> bool {
    let Some(reste) = url.strip_prefix("https://") else {
        return false;
    };
    let hote = reste.split(['/', '?', '#']).next().unwrap_or("");
    // Pas d'identifiants ni de port : « github.com@pirate.fr » ou « github.com:444 » refusés.
    HOTES.contains(&hote)
}

/// Agent HTTP sans redirection automatique (chaque saut est vérifié par `url_permise`) ;
/// `lecture` = attente maximale entre deux blocs reçus (s).
fn agent(lecture: u64) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(Duration::from_secs(8))
        .timeout_read(Duration::from_secs(lecture))
        .user_agent(&format!("{}-maj", dlcore::APP_NAME))
        .build()
}

/// GET qui suit au plus 5 redirections, chacune vérifiée par `url_permise`.
fn obtenir(agent: &ureq::Agent, url: &str) -> Result<ureq::Response, Erreur> {
    let mut url = url.to_string();
    for _ in 0..6 {
        if !url_permise(&url) {
            return Err("Download link refused (outside GitHub).");
        }
        let rep = agent
            .get(&url)
            .set(
                "Accept",
                "application/vnd.github+json, application/octet-stream",
            )
            .call()
            .map_err(|e| match e {
                ureq::Error::Status(403 | 429, _) => {
                    "GitHub is limiting requests for now: try again in an hour."
                }
                ureq::Error::Status(404, _) => "No release published yet.",
                ureq::Error::Status(..) => "GitHub returned an error.",
                ureq::Error::Transport(_) => "GitHub unreachable: check your Internet connection.",
            })?;
        if (300..400).contains(&rep.status()) {
            url = rep
                .header("location")
                .ok_or("Unreadable answer from GitHub.")?
                .to_string();
            continue;
        }
        return Ok(rep);
    }
    Err("Too many redirects: try again later.")
}

/// Dernière release du dépôt `proprio/nom`.
pub fn derniere(depot: &str) -> Result<Release, Erreur> {
    let url = format!("https://api.github.com/repos/{depot}/releases/latest");
    let mut d = Vec::new();
    obtenir(&agent(10), &url)?
        .into_reader()
        .take(4 << 20)
        .read_to_end(&mut d)
        .map_err(|_| "GitHub answer interrupted.")?;
    serde_json::from_slice(&d).map_err(|_| "Unreadable answer from GitHub.")
}

/// Petit fichier texte (empreintes), 1 Mo au plus.
pub fn texte(url: &str) -> Result<String, Erreur> {
    let mut t = String::new();
    obtenir(&agent(10), url)?
        .into_reader()
        .take(1 << 20)
        .read_to_string(&mut t)
        .map_err(|_| "Download interrupted: try again.")?;
    Ok(t)
}

/// Empreinte de `nom` dans un fichier d'empreintes : « <64 hex>  nom » ou « <64 hex> *nom »
/// par ligne (SHA2-256SUMS, checksums.sha256), ou les 64 caractères seuls (`nom` vide ou
/// fichier d'une seule ligne sans nom).
pub fn empreinte(t: &str, nom: &str) -> Result<String, Erreur> {
    let hex = |h: &str| h.len() == 64 && h.bytes().all(|c| c.is_ascii_hexdigit());
    for l in t.lines() {
        let mut m = l.split_whitespace();
        let (Some(h), n) = (m.next(), m.next()) else {
            continue;
        };
        let n = n.map(|n| n.trim_start_matches('*'));
        let n = n.map(|n| n.rsplit('/').next().unwrap_or(n));
        if hex(h) && (n.is_none() || n == Some(nom)) {
            return Ok(h.to_ascii_lowercase());
        }
    }
    Err("Unreadable checksum: nothing was changed.")
}

/// Télécharge `f` dans `dest` (écrit d'abord dans `dest.tmp`, renommé seulement si taille et
/// empreinte `sha` sont bonnes). `progres` reçoit les octets reçus ; `stop` interrompt.
pub fn telecharger(
    f: &Fichier,
    sha: &str,
    dest: &Path,
    stop: &AtomicBool,
    progres: impl FnMut(u64),
) -> Result<(), Erreur> {
    if f.size == 0 || f.size > 1 << 30 {
        return Err("Invalid published file.");
    }
    let tmp = suffixe(dest, ".tmp");
    let r = obtenir(&agent(20), &f.browser_download_url)
        .and_then(|rep| ecrire_verifie(rep.into_reader(), f.size, sha, &tmp, stop, progres));
    let r = r.and_then(|()| std::fs::rename(&tmp, dest).map_err(|_| "Cannot write the file."));
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

/// Copie `lecteur` dans `dest` (taille `taille` exacte, empreinte `sha`).
pub fn ecrire_verifie(
    lecteur: impl Read,
    taille: u64,
    sha: &str,
    dest: &Path,
    stop: &AtomicBool,
    mut progres: impl FnMut(u64),
) -> Result<(), Erreur> {
    let mut f = std::fs::File::create(dest).map_err(|_| "Cannot write the file.")?;
    let mut lecteur = lecteur.take(taille + 1);
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
    let mut tampon = vec![0u8; 64 << 10];
    let mut faits = 0u64;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err("Stopped.");
        }
        let n = lecteur
            .read(&mut tampon)
            .map_err(|_| "Download interrupted: try again.")?;
        if n == 0 {
            break;
        }
        ctx.update(&tampon[..n]);
        f.write_all(&tampon[..n])
            .map_err(|_| "Cannot write the file (disk full?): nothing was changed.")?;
        faits += n as u64;
        progres(faits);
    }
    if faits != taille || !dlcore::engine::hex(ctx.finish().as_ref()).eq_ignore_ascii_case(sha) {
        return Err("Damaged file received (checksum differs): nothing was changed.");
    }
    f.sync_all().map_err(|_| "Cannot write the file.")
}

/// `chemin` + `fin` (`fluxdl.exe` → `fluxdl.exe.ancien`).
pub fn suffixe(chemin: &Path, fin: &str) -> PathBuf {
    let mut s = chemin.as_os_str().to_owned();
    s.push(fin);
    PathBuf::from(s)
}

/// `a` plus récente que `b` (« 0.10.0 » > « 0.9.3 », préfixe « v » et suffixe « -… » ignorés).
pub fn plus_recente(a: &str, b: &str) -> bool {
    let nums = |s: &str| -> Vec<u64> {
        let s = s.trim().trim_start_matches('v');
        let s = s.split(['-', '+']).next().unwrap_or("");
        s.split('.').map(|p| p.parse().unwrap_or(0)).collect()
    };
    let (mut x, mut y) = (nums(a), nums(b));
    let n = x.len().max(y.len());
    x.resize(n, 0);
    y.resize(n, 0);
    x > y
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparaison_des_versions() {
        assert!(plus_recente("v0.2.0", "0.1.2"));
        assert!(plus_recente("0.10.0", "0.9.0"));
        assert!(plus_recente("v1.0", "0.99.9"));
        assert!(plus_recente("0.2.1", "0.2"));
        assert!(!plus_recente("v0.2.0", "0.2.0"));
        assert!(!plus_recente("0.1.9", "0.2.0"));
        assert!(!plus_recente("0.2.0-beta", "0.2.0"));
        assert!(!plus_recente("n'importe quoi", "0.1.0"));
    }

    #[test]
    fn liste_des_liens_permis() {
        for ok in [
            "https://api.github.com/repos/Sneakyquaza/fluxdl/releases/latest",
            "https://github.com/Sneakyquaza/fluxdl/releases/download/v0.2.0/fluxdl.exe",
            "https://objects.githubusercontent.com/github-production-release-asset/1?x=y",
            "https://release-assets.githubusercontent.com/github-production-release-asset/1",
        ] {
            assert!(url_permise(ok), "{ok}");
        }
        for non in [
            "http://github.com/Sneakyquaza/fluxdl/releases/download/v0.2.0/fluxdl.exe",
            "https://github.com.pirate.fr/fluxdl.exe",
            "https://github.com@pirate.fr/fluxdl.exe",
            "https://github.com:444/fluxdl.exe",
            "https://evilgithub.com/fluxdl.exe",
            "ftp://github.com/x",
            "file:///C:/fluxdl.exe",
        ] {
            assert!(!url_permise(non), "{non}");
        }
    }

    #[test]
    fn lecture_des_empreintes() {
        let h = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(
            empreinte(&format!("{}  fluxdl.exe\n", h.to_uppercase()), "fluxdl.exe").unwrap(),
            h
        );
        assert_eq!(empreinte(h, "fluxdl.exe").unwrap(), h);
        // SHA2-256SUMS de yt-dlp : plusieurs lignes, la bonne est choisie.
        let autre = "0".repeat(64);
        let sums = format!("{autre}  yt-dlp\n{h}  yt-dlp.exe\n{autre} *yt-dlp_x86.exe\n");
        assert_eq!(empreinte(&sums, "yt-dlp.exe").unwrap(), h);
        assert_eq!(empreinte(&sums, "yt-dlp_x86.exe").unwrap(), autre);
        assert!(empreinte(&sums, "absent.exe").is_err());
        assert!(empreinte("pas une empreinte", "x").is_err());
    }

    #[test]
    fn ecriture_verifiee() {
        let d = std::env::temp_dir().join(format!("fluxdl-test-gh-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let corps = b"nouvelle version".to_vec();
        let vrai =
            dlcore::engine::hex(ring::digest::digest(&ring::digest::SHA256, &corps).as_ref());
        let dest = d.join("f");
        let non = AtomicBool::new(false);
        let mut vus = 0;
        ecrire_verifie(&corps[..], corps.len() as u64, &vrai, &dest, &non, |n| {
            vus = n
        })
        .unwrap();
        assert_eq!(
            (std::fs::read(&dest).unwrap(), vus),
            (corps.clone(), corps.len() as u64)
        );
        // Empreinte fausse, fichier tronqué, arrêt demandé : refusés.
        let faux = "0".repeat(64);
        assert!(
            ecrire_verifie(&corps[..], corps.len() as u64, &faux, &dest, &non, |_| {}).is_err()
        );
        assert!(
            ecrire_verifie(&corps[..4], corps.len() as u64, &vrai, &dest, &non, |_| {}).is_err()
        );
        let oui = AtomicBool::new(true);
        assert!(
            ecrire_verifie(&corps[..], corps.len() as u64, &vrai, &dest, &oui, |_| {}).is_err()
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
