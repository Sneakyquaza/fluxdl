//! Historique des téléchargements finis, échoués ou annulés : `history.json` à côté de
//! queue.json, séparé de la file (gardé après « Retirer de la liste »), `MAX` entrées au plus,
//! sans cookies ni référent (en-têtes envoyés au site, parfois avec un jeton).

use crate::apres::{self, Categorie};
use crate::store::{self, Entry, Kind, Status};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const FICHIER: &str = "history.json";
/// Au-delà, les plus anciennes entrées partent.
pub const MAX: usize = 5000;

/// Un téléchargement fini, échoué ou annulé.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trace {
    /// L'entrée de la file telle qu'elle était (options vidéo comprises), sans cookies ni
    /// référent.
    pub entree: Entry,
    /// Fin (secondes Unix) ; durée du dernier passage (s) et octets reçus pendant ce passage.
    pub fin: u64,
    pub duree: u64,
    pub recus: u64,
    /// Fichier (ou dossier d'un torrent) écrit, si fini.
    #[serde(default)]
    pub chemin: Option<String>,
}

impl Trace {
    pub fn nom(&self) -> &str {
        self.entree.name.as_deref().unwrap_or(&self.entree.url)
    }

    /// Catégorie du tri par type (lot E) : vidéo → Vidéos (son seul → Musique), sinon d'après
    /// le nom.
    pub fn categorie(&self) -> Categorie {
        match &self.entree.kind {
            Kind::Video { quality, .. } if quality.audio() => Categorie::Musique,
            Kind::Video { .. } => Categorie::Videos,
            _ => apres::categorie(self.nom()),
        }
    }

    /// Fichier écrit : chemin noté à la fin, sinon dossier + nom d'un fichier fini (entrée d'avant
    /// l'historique).
    pub fn fichier(&self) -> Option<std::path::PathBuf> {
        let e = &self.entree;
        match (&self.chemin, &e.name) {
            (Some(c), _) => Some(c.into()),
            (None, Some(n)) if e.status == Status::Done => Some(Path::new(&e.dir).join(n)),
            _ => None,
        }
    }

    /// Vitesse moyenne du dernier passage (octets/s), si mesurable.
    pub fn vitesse(&self) -> Option<u64> {
        (self.duree > 0 && self.recus > 0).then(|| self.recus / self.duree)
    }
}

/// Note la fin d'un téléchargement : remplace la trace du même id (réessayé puis fini) ; la plus
/// ancienne part au-delà de `MAX`.
pub fn noter(
    h: &mut Vec<Trace>,
    e: &Entry,
    fin: u64,
    duree: u64,
    recus: u64,
    chemin: Option<String>,
) {
    let mut entree = e.clone();
    entree.cookies = None;
    entree.referrer = None;
    h.retain(|t| t.entree.id != e.id);
    h.push(Trace {
        entree,
        fin,
        duree,
        recus,
        chemin,
    });
    if h.len() > MAX {
        let n = h.len() - MAX;
        h.drain(..n);
    }
}

/// Lu entrée par entrée (comme la file) : une entrée illisible ne vide pas l'historique ; le
/// fichier d'origine est alors copié en `.bak`.
pub fn charger(d: &Path) -> Vec<Trace> {
    let f = d.join(FICHIER);
    let Ok(octets) = std::fs::read(&f) else {
        return Vec::new();
    };
    let valeurs: Vec<serde_json::Value> = serde_json::from_slice(&octets).unwrap_or_default();
    let h: Vec<Trace> = valeurs
        .iter()
        .filter_map(|v| serde_json::from_value(v.clone()).ok())
        .collect();
    if h.len() != valeurs.len() || valeurs.is_empty() && !octets.is_empty() {
        let _ = std::fs::copy(&f, d.join(format!("{FICHIER}.bak")));
    }
    h
}

pub fn sauver(d: &Path, h: &[Trace]) {
    store::save_in(d, FICHIER, &h);
}

/// Filtre d'état : Terminés, Échoués (annulés compris), Tous.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Filtre {
    Tous,
    Termines,
    Echecs,
}

/// Indices des traces qui passent la recherche (nom, lien, domaine ; sans casse), le filtre
/// d'état et la catégorie, les plus récentes d'abord (ou les plus anciennes).
pub fn chercher(
    h: &[Trace],
    texte: &str,
    filtre: Filtre,
    cat: Option<Categorie>,
    recentes: bool,
) -> Vec<usize> {
    let q = texte.trim().to_lowercase();
    let mut v: Vec<usize> = (0..h.len())
        .filter(|&i| {
            let t = &h[i];
            let etat = match filtre {
                Filtre::Tous => true,
                Filtre::Termines => t.entree.status == Status::Done,
                Filtre::Echecs => t.entree.status != Status::Done,
            };
            etat && cat.is_none_or(|c| t.categorie() == c)
                && (q.is_empty()
                    || t.nom().to_lowercase().contains(&q)
                    || t.entree.url.to_lowercase().contains(&q))
        })
        .collect();
    // Tri stable : à date égale, l'ordre d'arrivée.
    v.sort_by_key(|&i| h[i].fin);
    if recentes {
        v.reverse();
    }
    v
}

/// Entrée à remettre en file (Réessayer / Retélécharger) : mêmes options (vidéo, connexions,
/// limite, SHA-256, dossier), id neuf. Un fichier HTTP non fini dont le `.part` est encore là
/// reprend ; sinon il repart de zéro (nom choisi de nouveau par la sonde : jamais un fichier
/// fini écrasé).
pub fn relancer(t: &Trace, id: u64) -> Entry {
    let mut e = t.entree.clone();
    e.id = id;
    e.status = Status::Queued;
    if e.kind == Kind::Http {
        let reprise = t.entree.status != Status::Done
            && e.name
                .as_ref()
                .is_some_and(|n| dlcore::engine::part_path(&Path::new(&e.dir).join(n)).exists());
        if !reprise {
            e.name = None;
            e.size = None;
            e.done = 0;
        }
    } else {
        e.done = 0;
    }
    e
}

/// Date et heure lisibles (« 28/11/2026 14:05 », en anglais « 2026-11-28 14:05 ») ;
/// `decalage` : minutes à ajouter à l'heure UTC pour l'heure locale.
pub fn date(fin: u64, decalage: i64, fr: bool) -> String {
    let s = fin as i64 + decalage * 60;
    let (jours, m) = (s.div_euclid(86_400), s.rem_euclid(86_400) / 60);
    // Jours depuis 1970 → date civile (algorithme de H. Hinnant).
    let z = jours + 719_468;
    let ere = z.div_euclid(146_097);
    let j = z - ere * 146_097;
    let a = (j - j / 1460 + j / 36_524 - j / 146_096) / 365;
    let jda = j - (365 * a + a / 4 - a / 100);
    let mp = (5 * jda + 2) / 153;
    let jour = jda - (153 * mp + 2) / 5 + 1;
    let mois = if mp < 10 { mp + 3 } else { mp - 9 };
    let an = a + ere * 400 + i64::from(mois <= 2);
    if fr {
        format!("{jour:02}/{mois:02}/{an} {:02}:{:02}", m / 60, m % 60)
    } else {
        format!("{an}-{mois:02}-{jour:02} {:02}:{:02}", m / 60, m % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entree(id: u64, url: &str, nom: &str, status: Status) -> Entry {
        Entry {
            id,
            url: url.into(),
            dir: "/nulle-part".into(),
            name: Some(nom.into()),
            sha256: None,
            connections: 8,
            status,
            size: Some(1000),
            done: 1000,
            kind: Kind::Http,
            referrer: Some("https://site/page?jeton=REFERENT-SECRET".into()),
            cookies: Some("SID=COOKIE-SECRET".into()),
            speed_limit_mb: 0.0,
        }
    }

    /// Une trace par id (la dernière fin), plafond `MAX`, persistance sans cookies ni référent.
    #[test]
    fn noter_plafond_persistance_sans_cookies() {
        let d = std::env::temp_dir().join(format!("fluxdl-test-hist-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let mut h = Vec::new();
        noter(
            &mut h,
            &entree(
                1,
                "https://a/x.zip",
                "x.zip",
                Status::Failed("HTTP 500".into()),
            ),
            10,
            5,
            500,
            None,
        );
        noter(
            &mut h,
            &entree(1, "https://a/x.zip", "x.zip", Status::Done),
            20,
            4,
            1000,
            Some("/nulle-part/x.zip".into()),
        );
        assert_eq!(h.len(), 1);
        assert_eq!(
            (h[0].entree.status.clone(), h[0].fin, h[0].vitesse()),
            (Status::Done, 20, Some(250))
        );
        for i in 2..MAX as u64 + 10 {
            noter(
                &mut h,
                &entree(i, "https://a/y.zip", "y.zip", Status::Canceled),
                100 + i,
                0,
                0,
                None,
            );
        }
        assert_eq!(h.len(), MAX);
        // Les 9 plus anciennes sont parties.
        assert!(h.iter().all(|t| t.entree.id > 9));
        sauver(&d, &h);
        let texte = std::fs::read_to_string(d.join(FICHIER)).unwrap();
        assert!(!texte.contains("COOKIE-SECRET") && !texte.contains("REFERENT-SECRET"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = std::fs::metadata(d.join(FICHIER))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(m & 0o777, 0o600);
        }
        let lu = charger(&d);
        assert_eq!(lu.len(), MAX);
        assert_eq!(lu.last().unwrap().entree.id, h.last().unwrap().entree.id);
        // Fichier abîmé : copié en .bak, jamais perdu sans copie.
        std::fs::write(d.join(FICHIER), b"[{\"entree\":").unwrap();
        assert!(charger(&d).is_empty());
        assert_eq!(
            std::fs::read(d.join(format!("{FICHIER}.bak"))).unwrap(),
            b"[{\"entree\":"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn recherche_et_filtres() {
        let mut h = Vec::new();
        let mut vid = entree(
            3,
            "https://www.youtube.com/watch?v=abc",
            "Concert",
            Status::Done,
        );
        vid.kind = Kind::Video {
            quality: crate::video::Qualite::P1080,
            estimate: None,
            section: None,
            from_start: false,
            login: false,
            options: Default::default(),
        };
        for (e, fin) in [
            (
                entree(
                    1,
                    "https://releases.ubuntu.com/u.iso",
                    "ubuntu.iso",
                    Status::Done,
                ),
                30,
            ),
            (
                entree(
                    2,
                    "https://exemple.fr/doc.pdf",
                    "Rapport.PDF",
                    Status::Failed("x".into()),
                ),
                10,
            ),
            (vid, 20),
            (
                entree(4, "https://exemple.fr/a.zip", "a.zip", Status::Canceled),
                40,
            ),
        ] {
            noter(&mut h, &e, fin, 1, 1, None);
        }
        let ids = |v: Vec<usize>| v.iter().map(|&i| h[i].entree.id).collect::<Vec<_>>();
        assert_eq!(
            ids(chercher(&h, "", Filtre::Tous, None, true)),
            [4, 1, 3, 2]
        );
        assert_eq!(
            ids(chercher(&h, "", Filtre::Tous, None, false)),
            [2, 3, 1, 4]
        );
        assert_eq!(ids(chercher(&h, "", Filtre::Termines, None, true)), [1, 3]);
        assert_eq!(ids(chercher(&h, "", Filtre::Echecs, None, true)), [4, 2]);
        // Domaine, nom sans casse, lien.
        assert_eq!(
            ids(chercher(&h, " EXEMPLE.fr ", Filtre::Tous, None, true)),
            [4, 2]
        );
        assert_eq!(ids(chercher(&h, "rapport", Filtre::Tous, None, true)), [2]);
        assert_eq!(ids(chercher(&h, "v=abc", Filtre::Tous, None, true)), [3]);
        // Type : vidéo d'après sa sorte, sinon d'après l'extension.
        assert_eq!(
            ids(chercher(
                &h,
                "",
                Filtre::Tous,
                Some(Categorie::Videos),
                true
            )),
            [3]
        );
        assert_eq!(
            ids(chercher(
                &h,
                "",
                Filtre::Tous,
                Some(Categorie::Documents),
                true
            )),
            [2]
        );
        assert_eq!(
            ids(chercher(
                &h,
                "",
                Filtre::Echecs,
                Some(Categorie::Archives),
                true
            )),
            [4]
        );
        assert!(chercher(&h, "rien", Filtre::Tous, None, true).is_empty());
    }

    /// Réessayer : mêmes options vidéo, id neuf, en attente ; HTTP sans `.part` → de zéro.
    #[test]
    fn relancer_garde_les_options() {
        let mut v = entree(
            7,
            "https://www.youtube.com/watch?v=abc",
            "Concert",
            Status::Failed("x".into()),
        );
        let sortie = crate::video::Sortie {
            subtitles: true,
            sub_lang: Some("fr".into()),
            chapters: true,
            ..Default::default()
        };
        v.kind = Kind::Video {
            quality: crate::video::Qualite::P720,
            estimate: Some(5),
            section: Some((10, 20)),
            from_start: false,
            login: false,
            options: sortie,
        };
        v.speed_limit_mb = 5.0;
        let mut h = Vec::new();
        noter(&mut h, &v, 1, 1, 1, None);
        let e = relancer(&h[0], 42);
        assert_eq!((e.id, e.status.clone(), e.done), (42, Status::Queued, 0));
        assert_eq!(e.kind, h[0].entree.kind);
        assert_eq!((e.speed_limit_mb, e.cookies.as_deref()), (5.0, None));
        let d = std::env::temp_dir().join(format!("fluxdl-test-relance-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let mut f = entree(
            8,
            "https://a/gros.bin",
            "gros.bin",
            Status::Failed("x".into()),
        );
        f.dir = d.display().to_string();
        f.done = 300;
        noter(&mut h, &f, 2, 1, 1, None);
        // Pas de .part : de zéro, nom rechoisi.
        let e = relancer(&h[1], 43);
        assert_eq!((e.name.as_deref(), e.done), (None, 0));
        // .part encore là : reprise.
        std::fs::write(dlcore::engine::part_path(&d.join("gros.bin")), b"").unwrap();
        let e = relancer(&h[1], 44);
        assert_eq!((e.name.as_deref(), e.done), (Some("gros.bin"), 300));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn dates_lisibles() {
        // 2026-11-28 13:05 UTC.
        let s = 1_795_871_100;
        assert_eq!(date(s, 0, true), "28/11/2026 13:05");
        assert_eq!(date(s, 60, false), "2026-11-28 14:05");
        assert_eq!(date(0, 0, true), "01/01/1970 00:00");
        assert_eq!(date(951_782_400, 0, false), "2000-02-29 00:00");
    }
}
