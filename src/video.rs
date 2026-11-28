//! Vidéos par yt-dlp : formats lus avec `yt-dlp -J`, téléchargement confié à yt-dlp lui-même
//! (progression lue sur `--progress-template`), fusion par ffmpeg ; format en liens HTTP directs
//! (YouTube) : chaque partie par le moteur multi-connexions, repli sur yt-dlp seul. Windows : yt-dlp.exe,
//! ffmpeg et deno (JavaScript exigé par YouTube : sans lui, plus aucun format utilisable,
//! « Requested format is not available ») téléchargés depuis leurs releases GitHub officielles
//! (empreintes vérifiées) dans le dossier de données, après accord ; yt-dlp mis à jour chaque
//! semaine (`-U`). Linux : ceux du système.

use crate::github::{self, Erreur, Fichier};
use dlcore::engine::{self, Control, Live, Options, Outcome, Segment, WorkerStat};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

/// Erreur quand yt-dlp manque (Linux : à installer par le système).
pub const ABSENT: &str = "yt-dlp not found: install yt-dlp and ffmpeg.";

/// Vidéo lancée avec les cookies de connexion, qui ne sont plus là (l'appli a redémarré :
/// ils ne sont jamais écrits sur le disque).
pub const ERR_COOKIES: &str = "Login cookies are gone (fluxdl was restarted): click the fluxdl button on the video page again to resume.";

/// Sites vidéo connus : leurs liens vont droit au choix du format.
const HOTES_VIDEO: [&str; 14] = [
    "youtube.com",
    "youtu.be",
    "vimeo.com",
    "dailymotion.com",
    "twitch.tv",
    "tiktok.com",
    "twitter.com",
    "x.com",
    "instagram.com",
    "facebook.com",
    "reddit.com",
    "soundcloud.com",
    "bilibili.com",
    "rumble.com",
];

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Qualite {
    Best,
    P1080,
    P720,
    M4a,
    Mp3,
}

impl Qualite {
    pub const TOUTES: [Qualite; 5] = [
        Qualite::Best,
        Qualite::P1080,
        Qualite::P720,
        Qualite::M4a,
        Qualite::Mp3,
    ];

    pub fn audio(self) -> bool {
        matches!(self, Qualite::M4a | Qualite::Mp3)
    }

    fn hauteur(self) -> u32 {
        match self {
            Qualite::P1080 => 1080,
            Qualite::P720 => 720,
            _ => u32::MAX,
        }
    }

    /// Arguments de format pour yt-dlp.
    pub fn args(self) -> Vec<&'static str> {
        match self {
            Qualite::Best => vec!["-f", "bv*+ba/b"],
            // Conteneur : MP4 ou original selon `Sortie::mp4`.
            Qualite::P1080 => vec!["-f", "bv*[height<=1080]+ba/b[height<=1080]/b"],
            Qualite::P720 => vec!["-f", "bv*[height<=720]+ba/b[height<=720]/b"],
            // Pas de son seul (formats image + son ensemble) : son extrait du meilleur format.
            // M4a = son original sans conversion (M4A ou Opus, nom gardé pour queue.json).
            Qualite::M4a => vec!["-f", "ba/b", "-x"],
            Qualite::Mp3 => vec![
                "-f",
                "ba/b",
                "-x",
                "--audio-format",
                "mp3",
                "--audio-quality",
                "0",
            ],
        }
    }
}

/// Ce que `yt-dlp -J` dit d'une vidéo : titre, durée, hauteur max, taille estimée par choix.
#[derive(Clone, Debug, PartialEq)]
pub struct Infos {
    pub titre: String,
    pub duree: Option<f64>,
    pub hauteur_max: u32,
    /// (qualité, disponible, taille estimée).
    pub choix: Vec<(Qualite, bool, Option<u64>)>,
    /// Live en cours (`is_live`) : pas de section, « depuis le début » possible sur YouTube.
    pub en_direct: bool,
    /// Sous-titres proposés : ceux du site puis les automatiques, par code.
    pub sous_titres: Vec<SousTitre>,
}

/// Une langue de sous-titres (`subtitles` / `automatic_captions` de `yt-dlp -J`).
#[derive(Clone, Debug, PartialEq)]
pub struct SousTitre {
    /// Code yt-dlp (« fr », « en-US »), passé tel quel à `--sub-langs`.
    pub code: String,
    pub nom: String,
    /// Sous-titres automatiques (`--write-auto-subs`).
    pub auto: bool,
}

/// Langue de l'interface (« fr ») parmi les sous-titres : ceux du site d'abord, puis les
/// automatiques ; le code le plus court (« fr » avant « fr-CA »). Aucun : pas de sous-titres.
pub fn sous_titres_par_defaut<'a>(s: &'a [SousTitre], langue: &str) -> Option<&'a SousTitre> {
    let va = |c: &str| c == langue || c.strip_prefix(langue).is_some_and(|r| r.starts_with('-'));
    [false, true].into_iter().find_map(|auto| {
        s.iter()
            .filter(|p| p.auto == auto && va(&p.code))
            .min_by_key(|p| p.code.len())
    })
}

/// Ce qui s'ajoute à la vidéo (réglages par défaut et queue.json) : sous-titres, miniature,
/// chapitres (ffmpeg requis), conteneur.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sortie {
    /// Case « Sous-titres » ; langue choisie (code yt-dlp ; aucune = pas de sous-titres) et
    /// si ce sont les automatiques.
    pub subtitles: bool,
    pub sub_lang: Option<String>,
    pub sub_auto: bool,
    /// Fichier .srt à côté plutôt qu'intégrés à la vidéo.
    pub srt: bool,
    pub thumbnail: bool,
    pub chapters: bool,
    /// MP4 (H.264/AAC si possible) plutôt que l'original (MKV/WebM).
    pub mp4: bool,
}

impl Sortie {
    fn sous_titres(&self) -> Option<&str> {
        self.sub_lang.as_deref().filter(|_| self.subtitles)
    }

    /// Ajouts faits par yt-dlp après le téléchargement (passe finale après le moteur).
    fn ajouts(&self) -> bool {
        self.sous_titres().is_some() || self.thumbnail || self.chapters
    }
}

#[derive(Deserialize)]
struct Json {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    formats: Vec<Format>,
    #[serde(default)]
    is_live: Option<bool>,
    #[serde(default)]
    subtitles: Option<BTreeMap<String, Vec<Piste>>>,
    #[serde(default)]
    automatic_captions: Option<BTreeMap<String, Vec<Piste>>>,
}

/// Un format d'une langue de sous-titres (vtt, srv3…) : seul le nom sert.
#[derive(Deserialize)]
struct Piste {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct Format {
    #[serde(default)]
    height: Option<f64>,
    #[serde(default)]
    vcodec: Option<String>,
    #[serde(default)]
    acodec: Option<String>,
    #[serde(default)]
    filesize: Option<f64>,
    #[serde(default)]
    filesize_approx: Option<f64>,
    #[serde(default)]
    tbr: Option<f64>,
}

impl Format {
    fn taille(&self) -> Option<u64> {
        self.filesize
            .or(self.filesize_approx)
            .filter(|t| *t > 0.0)
            .map(|t| t as u64)
    }
    fn image(&self) -> bool {
        self.vcodec.as_deref() != Some("none") && self.height.is_some_and(|h| h > 0.0)
    }
    /// Son seul.
    fn son(&self) -> bool {
        self.vcodec.as_deref() == Some("none") && self.acodec.as_deref() != Some("none")
    }
}

/// `yt-dlp -J` → infos et tailles estimées (meilleure image sous la limite + meilleur son).
pub fn infos_depuis_json(d: &[u8]) -> Result<Infos, String> {
    let j: Json = serde_json::from_slice(d).map_err(|e| format!("yt-dlp: {e}"))?;
    let tbr = |f: &&Format| (f.tbr.unwrap_or(0.0) * 1000.0) as u64;
    let son = j.formats.iter().filter(|f| f.son()).max_by_key(tbr);
    let hauteur_max = j
        .formats
        .iter()
        .filter(|f| f.image())
        .filter_map(|f| f.height)
        .fold(0.0, f64::max) as u32;
    let a_du_son = son.is_some()
        || j.formats
            .iter()
            .any(|f| f.acodec.as_deref().is_some_and(|a| a != "none"));
    let choix = Qualite::TOUTES
        .iter()
        .map(|&q| {
            if q.audio() {
                return (q, a_du_son, son.and_then(Format::taille));
            }
            let h = q.hauteur();
            let image = j
                .formats
                .iter()
                .filter(|f| f.image() && f.height.unwrap_or(0.0) <= h as f64)
                .max_by_key(|f| ((f.height.unwrap_or(0.0) as u64), tbr(f)));
            let taille = image.and_then(|v| {
                let t = v.taille()?;
                // Image sans son : le meilleur son s'y ajoute (fusion par ffmpeg).
                if v.acodec.as_deref() == Some("none") {
                    Some(t + son.and_then(Format::taille).unwrap_or(0))
                } else {
                    Some(t)
                }
            });
            let dispo = image.is_some() && (h == u32::MAX || hauteur_max >= h);
            (q, dispo, taille)
        })
        .collect();
    let mut sous_titres = Vec::new();
    for (pistes, auto) in [(j.subtitles, false), (j.automatic_captions, true)] {
        for (code, p) in pistes.unwrap_or_default() {
            // Code donné à `--sub-langs` (expression régulière) : lettres, chiffres, tirets
            // seulement (écarte aussi « live_chat »).
            if code.is_empty()
                || code.len() > 20
                || !code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                continue;
            }
            let nom = p
                .iter()
                .find_map(|p| p.name.as_deref().map(str::trim).filter(|n| !n.is_empty()))
                .map_or_else(|| code.clone(), |n| n.chars().take(60).collect());
            sous_titres.push(SousTitre { code, nom, auto });
        }
    }
    Ok(Infos {
        titre: j.title.unwrap_or_else(|| "video".into()),
        duree: j.duration,
        hauteur_max,
        choix,
        en_direct: j.is_live == Some(true),
        sous_titres,
    })
}

/// Nom court d'une hauteur : 4320 → « 8K », 2160 → « 4K », 1080 → « 1080p ».
pub fn nom_hauteur(h: u32) -> String {
    match h {
        4320.. => "8K".into(),
        2160.. => "4K".into(),
        _ => format!("{h}p"),
    }
}

/// Lien d'un site vidéo connu (domaine ou sous-domaine).
pub fn site_video(url: &str) -> bool {
    let hote = url
        .split("://")
        .nth(1)
        .unwrap_or("")
        .split(['/', '?', '#', ':'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    HOTES_VIDEO
        .iter()
        .any(|h| hote == *h || hote.ends_with(&format!(".{h}")))
}

/// Lien vers plusieurs vidéos (playlist, chaîne, rediffusions d'une chaîne Twitch).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GenreListe {
    /// Playlist, chaîne ou rediffusions : la liste seule.
    Liste,
    /// Vidéo ouverte dans une playlist (`watch?v=…&list=…`) : demander laquelle.
    VideoDeListe,
}

/// Lien de liste et lien à donner à yt-dlp : chaîne YouTube sans onglet → son onglet Vidéos
/// (sinon yt-dlp rend la liste des onglets Vidéos / Live / Shorts, pas les vidéos).
pub fn liste(url: &str) -> Option<(GenreListe, String)> {
    let reste = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let reste = reste.split('#').next().unwrap_or("");
    let (hote, chemin) = reste.split_at(reste.find(['/', '?']).unwrap_or(reste.len()));
    let (chemin, requete) = chemin.split_once('?').unwrap_or((chemin, ""));
    let param = |n: &str| {
        requete
            .split('&')
            .find_map(|p| p.strip_prefix(n)?.strip_prefix('='))
            .filter(|v| !v.is_empty())
    };
    let s: Vec<&str> = chemin.split('/').filter(|s| !s.is_empty()).collect();
    let nom = |n: &str| {
        (1..=100).contains(&n.len())
            && n.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.%".contains(&b))
    };
    let lien = |u: String| Some((GenreListe::Liste, u));
    match crate::cookies::site_de(url).as_str() {
        "youtube.com" => {
            let liste = param("list").is_some();
            let hote = hote.to_ascii_lowercase();
            match s[..] {
                ["playlist"] if liste => lien(url.into()),
                ["watch"] if liste && param("v").is_some() => {
                    Some((GenreListe::VideoDeListe, url.into()))
                }
                [_] if liste && (hote == "youtu.be" || hote.ends_with(".youtu.be")) => {
                    Some((GenreListe::VideoDeListe, url.into()))
                }
                [c] if c.starts_with('@') && nom(&c[1..]) => {
                    lien(format!("https://www.youtube.com/{c}/videos"))
                }
                ["channel" | "c", c] if nom(c) => {
                    lien(format!("https://www.youtube.com/{}/{c}/videos", s[0]))
                }
                [c, "videos" | "shorts" | "streams"] if c.starts_with('@') && nom(&c[1..]) => {
                    lien(url.into())
                }
                ["channel" | "c", c, "videos" | "shorts" | "streams"] if nom(c) => lien(url.into()),
                _ => None,
            }
        }
        // Rediffusions publiques d'une chaîne (`twitch.tv/videos/N` = une seule).
        "twitch.tv" => match s[..] {
            [c, "videos"]
                if c != "videos"
                    && (3..=25).contains(&c.len())
                    && c.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') =>
            {
                lien(url.into())
            }
            _ => None,
        },
        _ => None,
    }
}

/// Entrées lues à la fois (« Charger plus » lit les suivantes).
pub const LISTE_MAX: usize = 500;

pub const ERR_LISTE_PRIVEE: &str = "This playlist is private: only its owner can see it.";
pub const ERR_LISTE_INTROUVABLE: &str = "Playlist or channel not found: check the link.";
pub const ERR_LISTE_VIDE: &str = "This list has no video.";

/// Vidéos d'une playlist ou d'une chaîne (`yt-dlp --flat-playlist -J`, sans les formats).
#[derive(Clone, Debug, PartialEq)]
pub struct Playlist {
    pub titre: String,
    pub videos: Vec<VideoListe>,
    /// D'autres entrées suivent (plus de `LISTE_MAX`).
    pub suite: bool,
    /// Entrées lues depuis le début de la liste, écartées comprises (début de la suite).
    pub lues: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VideoListe {
    pub url: String,
    pub titre: String,
    pub duree: Option<f64>,
    /// Place dans la liste (1 = la première).
    pub numero: usize,
}

#[derive(Deserialize)]
struct JsonListe {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    entries: Vec<Option<JsonEntree>>,
}

#[derive(Deserialize)]
struct JsonEntree {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
}

/// `--flat-playlist -J` lu à partir de l'entrée `debut` (0 = la première), `LISTE_MAX + 1`
/// demandées : la dernière dit seulement qu'il y a une suite. Écartés : entrées sans lien
/// http(s) et listes dans la liste (onglets d'une chaîne, playlists d'un onglet).
pub fn playlist_depuis_json(d: &[u8], debut: usize) -> Result<Playlist, String> {
    let j: JsonListe = serde_json::from_slice(d).map_err(|e| format!("yt-dlp: {e}"))?;
    let suite = j.entries.len() > LISTE_MAX;
    let lues = debut + j.entries.len().min(LISTE_MAX);
    let videos = j
        .entries
        .into_iter()
        .take(LISTE_MAX)
        .enumerate()
        .filter_map(|(i, e)| {
            let e = e?;
            let url = e.url.filter(|u| {
                (u.starts_with("https://") || u.starts_with("http://")) && liste(u).is_none()
            })?;
            let titre = e
                .title
                .map(|t| t.trim().chars().take(200).collect::<String>())
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| url.clone());
            Some(VideoListe {
                url,
                titre,
                duree: e.duration,
                numero: debut + i + 1,
            })
        })
        .collect();
    Ok(Playlist {
        titre: j
            .title
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "playlist".into()),
        videos,
        suite,
        lues,
    })
}

/// Liste des vidéos d'une playlist / chaîne sans rien télécharger (bloquant : à lancer dans un
/// fil). `stop` : yt-dlp tué (fenêtre fermée). Erreurs connues traduites (privée, introuvable,
/// vide).
pub fn lire_liste(
    o: &Outils,
    url: &str,
    cookies: Option<&Path>,
    debut: usize,
    stop: &AtomicBool,
) -> Result<Playlist, String> {
    let yt = o.ytdlp.as_ref().ok_or("yt-dlp missing")?;
    let mut c = commande(yt);
    c.args(["--flat-playlist", "-J"]);
    args_communs(&mut c, o, cookies);
    // Après `--no-playlist` des arguments communs : la dernière l'emporte.
    let mut fils = c
        .arg("--yes-playlist")
        .arg("--playlist-start")
        .arg((debut + 1).to_string())
        .arg("--playlist-end")
        .arg((debut + LISTE_MAX + 1).to_string())
        .arg("--")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("yt-dlp: {e}"))?;
    let lire = |r: Option<Box<dyn Read + Send>>| {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut s = Vec::new();
            if let Some(mut r) = r {
                let _ = r.read_to_end(&mut s);
            }
            let _ = tx.send(s);
        });
        rx
    };
    let sortie = lire(fils.stdout.take().map(|r| Box::new(r) as _));
    let err = lire(fils.stderr.take().map(|r| Box::new(r) as _));
    let s = loop {
        if stop.load(Ordering::Relaxed) {
            tuer(&mut fils);
            return Err("stopped".into());
        }
        match sortie.recv_timeout(Duration::from_millis(100)) {
            Ok(s) => break s,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break Vec::new(),
        }
    };
    let ok = fils.wait().is_ok_and(|e| e.success());
    let err = String::from_utf8_lossy(&err.recv().unwrap_or_default()).into_owned();
    if !ok {
        let e = derniere_erreur(&err);
        let m = e.to_ascii_lowercase();
        return Err(if m.contains("private") {
            ERR_LISTE_PRIVEE.into()
        } else if m.contains("does not exist") || m.contains("not found") || m.contains("404") {
            ERR_LISTE_INTROUVABLE.into()
        } else {
            e
        });
    }
    let p = playlist_depuis_json(&s, debut)?;
    if debut == 0 && p.videos.is_empty() {
        return Err(ERR_LISTE_VIDE.into());
    }
    Ok(p)
}

/// Nom de sous-dossier pour une playlist : nom de fichier sûr (Windows et Linux), 100 octets
/// au plus, sans point ni espace final ; vide → « playlist ».
pub fn nom_dossier(titre: &str) -> String {
    let mut n = engine::sanitize(titre);
    if n.len() > 100 {
        let mut coupe = 100;
        while !n.is_char_boundary(coupe) {
            coupe -= 1;
        }
        n.truncate(coupe);
    }
    // Windows refuse un point ou une espace final ; nom réservé (« NUL ») revérifié après coupe.
    let n = engine::sanitize(n.trim_end_matches(|c: char| c == '.' || c.is_whitespace()));
    if n.is_empty() {
        "playlist".into()
    } else {
        n
    }
}

/// Ligne de progression (`--progress-template`) : « fluxdl <reçus> <total> <estimé> ».
pub fn progression(l: &str) -> Option<(u64, Option<u64>)> {
    let mut m = l.strip_prefix("fluxdl ")?.split_whitespace();
    let n = |s: Option<&str>| s.and_then(|s| s.parse::<f64>().ok()).map(|v| v as u64);
    let recus = n(m.next())?;
    let (total, estime) = (n(m.next()), n(m.next()));
    Some((recus, total.or(estime)))
}

/// « 1:02:03 », « 2:03 », « 45 » → secondes (minutes et secondes < 60 après le premier champ).
pub fn temps(s: &str) -> Option<u32> {
    let p: Vec<&str> = s.trim().split(':').collect();
    if p.is_empty() || p.len() > 3 {
        return None;
    }
    let mut t = 0u32;
    for (i, x) in p.iter().enumerate() {
        if x.is_empty() || x.len() > 6 || !x.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let n: u32 = x.parse().ok()?;
        if i > 0 && n >= 60 {
            return None;
        }
        t = t.checked_mul(60)?.checked_add(n)?;
    }
    Some(t)
}

/// Secondes → « 01:02:03 ».
pub fn hms(t: u32) -> String {
    format!("{:02}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60)
}

/// Ce que l'utilisateur a choisi pour une vidéo (gardé dans queue.json, sans les cookies).
#[derive(Clone, Debug, PartialEq)]
pub struct Tache {
    pub qualite: Qualite,
    pub estimation: Option<u64>,
    /// Passage seulement (début, fin en secondes ; `--download-sections`, ffmpeg requis).
    pub section: Option<(u32, u32)>,
    /// Live YouTube en cours : depuis son début (`--live-from-start`) plutôt que maintenant.
    pub debut: bool,
    pub sortie: Sortie,
}

impl Tache {
    /// Arguments propres à la tâche : format, conteneur, sous-titres, miniature, chapitres,
    /// passage, live, nom du fichier.
    fn args(&self) -> Vec<String> {
        let mut a: Vec<String> = self.qualite.args().into_iter().map(String::from).collect();
        let s = &self.sortie;
        let mut ajoute = |v: &[&str]| a.extend(v.iter().map(|x| x.to_string()));
        if !self.qualite.audio() {
            if s.mp4 {
                ajoute(&[
                    "-S",
                    "vcodec:h264,acodec:m4a",
                    "--merge-output-format",
                    "mp4",
                ]);
            }
            if let Some(l) = s.sous_titres() {
                let ecrire = if s.sub_auto {
                    "--write-auto-subs"
                } else {
                    "--write-subs"
                };
                if s.srt {
                    ajoute(&[ecrire, "--sub-langs", l, "--convert-subs", "srt"]);
                } else {
                    // Sans `--write-subs`, yt-dlp efface le fichier une fois intégré.
                    if s.sub_auto {
                        ajoute(&[ecrire]);
                    }
                    ajoute(&["--embed-subs", "--sub-langs", l]);
                }
            }
        }
        if s.thumbnail {
            ajoute(&["--embed-thumbnail"]);
        }
        if s.chapters {
            ajoute(&["--embed-chapters"]);
        }
        let mut nom = "%(title).150B [%(id)s]".to_string();
        if let Some((de, a_)) = self.section {
            a.extend([
                "--download-sections".into(),
                format!("*{}-{}", hms(de), hms(a_)),
            ]);
            // Le passage n'écrase pas la vidéo entière (ni un autre passage).
            nom += &format!(
                " {}-{}",
                hms(de).replace(':', "."),
                hms(a_).replace(':', ".")
            );
        }
        if self.debut {
            a.push("--live-from-start".into());
        }
        a.extend(["-o".into(), format!("{nom}.%(ext)s")]);
        a
    }
}

/// « Depuis le début » d'un live : YouTube seulement (Twitch enregistre à partir de maintenant).
pub fn debut_possible(url: &str) -> bool {
    crate::cookies::site_de(url) == "youtube.com"
}

const MODELE: &str = "download:fluxdl %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s";

/// Emplacements de yt-dlp, ffmpeg (dossier de ffmpeg, pour `--ffmpeg-location`) et deno.
#[derive(Clone, Debug, Default)]
pub struct Outils {
    pub ytdlp: Option<PathBuf>,
    pub ffmpeg: Option<PathBuf>,
    pub deno: Option<PathBuf>,
}

impl Outils {
    pub fn complets(&self) -> bool {
        self.ytdlp.is_some() && self.ffmpeg.is_some() && self.deno.is_some()
    }
}

pub fn dossier_outils() -> PathBuf {
    crate::store::data_dir().join("tools")
}

const EXE: &str = if cfg!(windows) { ".exe" } else { "" };

/// Programme dans le PATH.
fn dans_path(nom: &str) -> Option<PathBuf> {
    let p = std::env::var_os("PATH")?;
    std::env::split_paths(&p)
        // Dossiers absolus seulement (une entrée vide ou « . » = dossier courant).
        .filter(|d| d.is_absolute())
        .map(|d| d.join(format!("{nom}{EXE}")))
        .find(|f| f.is_file())
}

/// Les nôtres (dossier de données) d'abord, sinon ceux du système.
pub fn trouver() -> Outils {
    let d = dossier_outils();
    let yt = d.join(format!("yt-dlp{EXE}"));
    let ff = d.join("ffmpeg").join("bin");
    let deno = d.join(format!("deno{EXE}"));
    Outils {
        ytdlp: yt.is_file().then_some(yt).or_else(|| dans_path("yt-dlp")),
        ffmpeg: ff
            .join(format!("ffmpeg{EXE}"))
            .is_file()
            .then_some(ff)
            .or_else(|| dans_path("ffmpeg").and_then(|f| f.parent().map(Path::to_path_buf))),
        deno: deno.is_file().then_some(deno).or_else(|| dans_path("deno")),
    }
}

use crate::systeme::commande;

/// Arguments communs : aucun fichier de config ni greffon (un `yt-dlp.conf` déposé dans le
/// dossier de téléchargement ou le dossier courant pourrait contenir `--exec`), sortie UTF-8,
/// environnement JavaScript pour YouTube, ffmpeg.
fn args_communs(c: &mut Command, o: &Outils, cookies: Option<&Path>) {
    c.arg("--ignore-config");
    // Notre copie (Windows) est à jour ; un yt-dlp système d'avant 2025.02 ne connaît pas
    // l'option (dossier courant sûr : pas de greffon pris dans Téléchargements).
    if o.ytdlp
        .as_ref()
        .is_some_and(|y| y.starts_with(dossier_outils()))
    {
        c.arg("--no-plugin-dirs");
    }
    c.env("PYTHONIOENCODING", "utf-8");
    c.args(["--encoding", "utf-8", "--no-playlist", "--js-runtimes"]);
    // deno par son chemin complet (le nôtre n'est pas dans le PATH), sinon node du système
    match &o.deno {
        Some(d) => c.arg(format!("deno:{}", d.display())),
        None => c.arg("deno"),
    };
    c.args(["--js-runtimes", "node"]);
    if let Some(f) = &o.ffmpeg {
        c.arg("--ffmpeg-location").arg(f);
    }
    if let Some(f) = cookies {
        c.arg("--cookies").arg(f);
    }
}

/// `yt-dlp -J` (bloquant : à lancer dans un fil).
/// `cookies` : fichier Netscape temporaire (vidéo réservée aux abonnés / membres).
pub fn lire_infos(o: &Outils, url: &str, cookies: Option<&Path>) -> Result<Infos, String> {
    let yt = o.ytdlp.as_ref().ok_or("yt-dlp missing")?;
    let mut c = commande(yt);
    c.arg("-J");
    args_communs(&mut c, o, cookies);
    let sortie = c
        .arg("--")
        .arg(url)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("yt-dlp: {e}"))?;
    if !sortie.status.success() {
        return Err(derniere_erreur(&String::from_utf8_lossy(&sortie.stderr)));
    }
    infos_depuis_json(&sortie.stdout)
}

/// Dernière ligne « ERROR: … » de yt-dlp (sinon la dernière ligne).
fn derniere_erreur(err: &str) -> String {
    let l = err.lines().rev();
    let e = l
        .clone()
        .find(|l| l.starts_with("ERROR:"))
        .or_else(|| err.lines().rev().find(|l| !l.trim().is_empty()))
        .unwrap_or("yt-dlp failed");
    e.trim_start_matches("ERROR:")
        .trim()
        .chars()
        .take(300)
        .collect()
}

/// Télécharge avec yt-dlp dans `dir` ; arrêt = processus tué, yt-dlp reprend ses `.part` au
/// prochain lancement (`--continue` ; pas pour un passage ni un live, recommencés).
/// Progression : un seul « morceau » dans `Live` (fichier puis son).
/// Avant : format en liens HTTP directs (YouTube…) → `par_le_moteur` sur `connexions`, et
/// yt-dlp seulement s'il échoue. Sites à fragments (HLS de Twitch…) : `connexions` fragments
/// en parallèle (16 au plus).
pub fn telecharger(
    o: &Outils,
    url: &str,
    dir: &Path,
    tache: &Tache,
    cookies: Option<&Path>,
    connexions: usize,
    ctl: &Arc<Control>,
) -> Result<Outcome, String> {
    let yt = o.ytdlp.as_ref().ok_or("yt-dlp missing")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // Sans ffmpeg, yt-dlp ne sait ni intégrer ni convertir : ni sous-titres, ni miniature, ni
    // chapitres (il échouerait).
    let sans_ffmpeg;
    let tache = if o.ffmpeg.is_none() {
        sans_ffmpeg = Tache {
            sortie: Sortie {
                mp4: tache.sortie.mp4,
                ..Default::default()
            },
            ..tache.clone()
        };
        &sans_ffmpeg
    } else {
        tache
    };
    // Fichier fini par le moteur : yt-dlp le retrouve (« already downloaded »), ne le
    // retélécharge pas et n'y fait que les ajouts (sous-titres, miniature, chapitres).
    let mut deja = None;
    // raccourci : son seul (extraction -x), passage et live restent à yt-dlp ; les passer au
    // moteur si les liens directs donnent satisfaction.
    if tache.section.is_none() && !tache.debut && !tache.qualite.audio() {
        if let Some(mut d) = liens_directs(o, url, dir, tache, cookies, ctl) {
            let ajouts = tache.sortie.ajouts();
            // Pas de miniature dans un webm : yt-dlp fusionne alors en mkv, la fusion maison
            // aussi (même nom que celui que cherchera la passe finale).
            if tache.sortie.thumbnail
                && d.parties.len() == 2
                && d.fichier.extension().is_some_and(|e| e == "webm")
            {
                d.fichier.set_extension("mkv");
            }
            // Déjà là avec des ajouts demandés : passe finale arrêtée avant la fin, la refaire.
            if ajouts && d.fichier.is_file() {
                deja = Some(d.fichier.clone());
            } else {
                match par_le_moteur(&d, o.ffmpeg.as_deref(), connexions, ctl) {
                    Ok(Outcome::Done(f)) if ajouts && f == d.fichier => deja = Some(f),
                    Ok(r) => return Ok(r),
                    Err(e) => {
                        // Plages refusées, lien expiré… : yt-dlp seul, sans échec visible.
                        eprintln!("fluxdl: direct video links failed ({e}), yt-dlp alone");
                        for f in temporaires(&d) {
                            let _ = std::fs::remove_file(f);
                        }
                        *ctl.lock() = Live::default();
                    }
                }
            }
        }
        if ctl.stop.load(Ordering::Relaxed) {
            return Ok(Outcome::Stopped);
        }
    }
    let taille = tache.estimation;
    let mut c = commande(yt);
    c.args(tache.args());
    args_communs(&mut c, o, cookies);
    c.args(["--newline", "--progress", "--no-simulate", "--continue"])
        .arg("--concurrent-fragments")
        .arg(connexions.clamp(1, 16).to_string())
        .args(limite_ytdlp(ctl))
        .args(["--progress-template", MODELE])
        .args(["--print", "after_move:fluxdl-fichier %(filepath)s"])
        .arg("-P")
        .arg(dir)
        .arg("--")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut fils = c.spawn().map_err(|e| format!("yt-dlp: {e}"))?;
    let (tx, rx) = mpsc::channel();
    let sortie = fils.stdout.take().map(BufReader::new);
    let tx2 = tx.clone();
    std::thread::spawn(move || lignes(sortie, |l| tx2.send((false, l)).is_ok()));
    let err = fils.stderr.take().map(BufReader::new);
    std::thread::spawn(move || lignes(err, |l| tx.send((true, l)).is_ok()));
    {
        let mut l = ctl.lock();
        l.size = taille.or(l.size);
        l.workers = vec![WorkerStat::default()];
    }
    // Plusieurs fichiers de suite (image puis son) : les octets des précédents s'ajoutent.
    let (mut base, mut dernier, mut dernier_total) = (0u64, 0u64, 0u64);
    let (mut fichier, mut erreurs) = (None, String::new());
    loop {
        if ctl.stop.load(Ordering::Relaxed) {
            tuer(&mut fils);
            return Ok(Outcome::Stopped);
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok((true, l)) => {
                erreurs.push_str(&l);
                erreurs.push('\n');
            }
            Ok((false, l)) => {
                if let Some(f) = l.strip_prefix("fluxdl-fichier ") {
                    fichier = Some(PathBuf::from(f.trim()));
                } else if let Some((recus, total)) = progression(&l) {
                    if recus + (1 << 20) < dernier {
                        base += dernier_total.max(dernier);
                    }
                    dernier = recus;
                    dernier_total = total.unwrap_or(recus);
                    let mut lv = ctl.lock();
                    let fait = base + recus;
                    let fin = lv.size.unwrap_or(0).max(base + dernier_total).max(fait);
                    lv.size = Some(fin);
                    lv.received = fait;
                    lv.workers[0].bytes = fait;
                    lv.workers[0].segment = Some(0);
                    lv.segments = vec![Segment {
                        start: 0,
                        pos: fait,
                        end: fin,
                        worker: Some(0),
                        stolen: false,
                    }];
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let ok = fils.wait().map(|s| s.success()).unwrap_or(false);
    match (ok, fichier) {
        (true, Some(f)) => {
            let mut lv = ctl.lock();
            let t = std::fs::metadata(&f).map(|m| m.len()).ok();
            lv.size = t.or(lv.size);
            for s in &mut lv.segments {
                s.pos = s.end;
            }
            Ok(Outcome::Done(f))
        }
        (true, None) => Ok(Outcome::Done(deja.unwrap_or_else(|| dir.to_path_buf()))),
        (false, _) => match deja {
            // La vidéo est complète : un ajout raté (miniature…) ne la fait pas échouer.
            Some(f) => {
                eprintln!(
                    "fluxdl: yt-dlp final pass failed ({}), video kept without extras",
                    derniere_erreur(&erreurs)
                );
                Ok(Outcome::Done(f))
            }
            None => Err(derniere_erreur(&erreurs)),
        },
    }
}

/// Plus grande plage par requête vers googlevideo (YouTube bride ou refuse les grosses plages).
const PLAGE_GOOGLEVIDEO: u64 = 10 << 20;

/// Une partie (image, son, ou les deux) d'un format choisi par yt-dlp.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
struct Partie {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    protocol: Option<String>,
    /// En-têtes que yt-dlp enverrait pour ce format (Cookie compris s'il l'y met) : les seuls
    /// envoyés, et seulement à l'hôte du lien (`engine::run`) ; jamais les cookies de l'extension.
    #[serde(default)]
    http_headers: BTreeMap<String, String>,
    /// Taille annoncée par yt-dlp : jauge unique sur toutes les parties dès le début.
    #[serde(default)]
    filesize: Option<f64>,
    #[serde(default)]
    filesize_approx: Option<f64>,
}

impl Partie {
    fn taille(&self) -> Option<u64> {
        self.filesize
            .or(self.filesize_approx)
            .filter(|t| *t > 0.0)
            .map(|t| t as u64)
    }
}

#[derive(Deserialize)]
struct Choisi {
    #[serde(default)]
    is_live: Option<bool>,
    #[serde(default)]
    requested_formats: Option<Vec<Partie>>,
    #[serde(flatten)]
    seul: Partie,
}

/// Format choisi en liens HTTP directs : parties (image puis son, ou une seule) et fichier final.
#[derive(Debug, PartialEq)]
struct Directs {
    parties: Vec<Partie>,
    fichier: PathBuf,
}

/// Sortie de `--print filename --print "%()j"` → liens directs, ou `None` (fragments HLS/DASH,
/// live, lien qui n'est pas en http(s), nom inutilisable).
fn directs(sortie: &str, dir: &Path) -> Option<Directs> {
    let mut l = sortie.lines().filter(|l| !l.trim().is_empty());
    // Nom seulement : le fichier reste dans `dir`, quoi que dise la sortie.
    let nom = Path::new(l.next()?.trim()).file_name()?.to_owned();
    let c: Choisi = serde_json::from_str(l.next()?).ok()?;
    if c.is_live == Some(true) {
        return None;
    }
    let parties = c.requested_formats.unwrap_or_else(|| vec![c.seul]);
    let direct = |p: &Partie| {
        matches!(p.protocol.as_deref(), Some("https" | "http"))
            && p.url
                .as_deref()
                .is_some_and(|u| u.starts_with("https://") || u.starts_with("http://"))
    };
    if parties.is_empty() || parties.len() > 2 || !parties.iter().all(direct) {
        return None;
    }
    Some(Directs {
        parties,
        fichier: dir.join(nom),
    })
}

/// yt-dlp choisit le format (mêmes arguments que le téléchargement) sans rien télécharger.
/// Arrêt demandé : yt-dlp est tué, `None`.
fn liens_directs(
    o: &Outils,
    url: &str,
    dir: &Path,
    tache: &Tache,
    cookies: Option<&Path>,
    ctl: &Control,
) -> Option<Directs> {
    let mut c = commande(o.ytdlp.as_ref()?);
    c.args(tache.args());
    args_communs(&mut c, o, cookies);
    let mut fils = c
        .args(["--print", "filename", "--print", "%()j", "-P"])
        .arg(dir)
        .arg("--")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let (tx, rx) = mpsc::channel();
    let mut sortie = fils.stdout.take();
    std::thread::spawn(move || {
        let mut s = Vec::new();
        if let Some(r) = &mut sortie {
            let _ = r.read_to_end(&mut s);
        }
        let _ = tx.send(s);
    });
    let s = loop {
        if ctl.stop.load(Ordering::Relaxed) {
            tuer(&mut fils);
            return None;
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(s) => break s,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                tuer(&mut fils);
                return None;
            }
        }
    };
    fils.wait()
        .is_ok_and(|e| e.success())
        .then(|| directs(&String::from_utf8_lossy(&s), dir))?
}

/// Noms des fichiers de la partie `i` sur `n` (`n` = 1 : directement le fichier final).
fn nom_partie(d: &Directs, i: usize) -> String {
    let nom = d.fichier.file_name().unwrap_or_default().to_string_lossy();
    match d.parties.len() {
        1 => nom.into_owned(),
        _ => format!("{nom}.fluxdl{i}"),
    }
}

/// Fusion des parties avant renommage : même extension que le fichier final (choix du conteneur).
fn fusion(d: &Directs) -> PathBuf {
    let ext = d.fichier.extension().unwrap_or("mkv".as_ref());
    d.fichier
        .with_extension(format!("fluxdl.{}", ext.to_string_lossy()))
}

/// Fichiers de travail à effacer si les liens directs échouent (jamais le fichier final).
fn temporaires(d: &Directs) -> Vec<PathBuf> {
    let dir = d.fichier.parent().unwrap_or(Path::new("."));
    let mut v = vec![fusion(d)];
    for i in 0..d.parties.len() {
        let t = dir.join(nom_partie(d, i));
        v.extend([engine::part_path(&t), engine::state_path(&t)]);
        if d.parties.len() > 1 {
            v.push(t);
        }
    }
    v
}

/// Jauge unique sur toutes les parties, dans l'ordre de téléchargement (son d'abord) : finies
/// pleines, celle en cours (`en_cours`) d'après son moteur (`l` : ses connexions), suivantes
/// vides ; `avant` = octets reçus par les parties précédentes de cette session. Nom final affiché.
fn jauge(ctl: &Control, nom: &str, tailles: &[Option<u64>], en_cours: usize, l: &Live, avant: u64) {
    let mut segments = Vec::new();
    let mut debut = 0;
    for i in (0..tailles.len()).rev() {
        let t = tailles[i].unwrap_or(0);
        let vide = Segment {
            start: debut,
            pos: debut,
            end: debut + t,
            ..Default::default()
        };
        match i.cmp(&en_cours) {
            std::cmp::Ordering::Greater => segments.push(Segment {
                pos: debut + t,
                ..vide
            }),
            std::cmp::Ordering::Equal => segments.extend(l.segments.iter().map(|s| Segment {
                start: s.start + debut,
                pos: s.pos + debut,
                end: s.end + debut,
                ..*s
            })),
            std::cmp::Ordering::Less => segments.push(vide),
        }
        debut += t;
    }
    let mut v = ctl.lock();
    v.file_name = Some(nom.to_string());
    // Taille inconnue tant qu'une partie n'a ni sonde ni estimation (la jauge ne recule pas).
    v.size = tailles.iter().copied().sum();
    v.ranges = true;
    v.segments = segments;
    v.workers = l.workers.clone();
    v.steals = l.steals;
    v.received = avant + l.received;
    v.restarted |= l.restarted;
    v.verifying = l.verifying;
}

/// `--limit-rate` : la plus basse des limites de l'entrée au lancement (yt-dlp ne la change pas
/// en cours ; une nouvelle limite vaut au prochain lancement).
/// raccourci : la limite globale n'est pas partagée avec les autres téléchargements pendant
/// que yt-dlp tourne seul ; la lui faire suivre si l'écart gêne.
fn limite_ytdlp(ctl: &Control) -> Vec<String> {
    match ctl
        .limites
        .iter()
        .map(|s| s.debit())
        .filter(|&d| d > 0)
        .min()
    {
        Some(d) => vec!["--limit-rate".into(), d.to_string()],
        None => Vec::new(),
    }
}

/// Chaque partie par le moteur (plages exigées, jauges par connexion), son d'abord : la jauge
/// finit sur l'image, la plus grosse ; puis fusion par ffmpeg (`-c copy`). Arrêt : les `.part`
/// et les parties finies restent, la reprise repart de là (liens neufs, même fichier distant).
fn par_le_moteur(
    d: &Directs,
    ffmpeg: Option<&Path>,
    connexions: usize,
    ctl: &Arc<Control>,
) -> Result<Outcome, String> {
    let dir = d.fichier.parent().unwrap_or(Path::new("."));
    let n = d.parties.len();
    let ff = match ffmpeg {
        Some(f) => f.join(format!("ffmpeg{EXE}")),
        None if n > 1 => return Err("ffmpeg missing".into()),
        None => PathBuf::new(),
    };
    let nom = d.fichier.file_name().unwrap_or_default().to_string_lossy();
    let mut tailles: Vec<_> = d.parties.iter().map(Partie::taille).collect();
    let mut faits = vec![PathBuf::new(); n];
    let mut avant = 0;
    jauge(ctl, &nom, &tailles, n - 1, &Live::default(), 0);
    for (i, p) in d.parties.iter().enumerate().rev() {
        let cible = dir.join(nom_partie(d, i));
        // Reprise : partie finie (renommée depuis son .part) lors d'un essai précédent.
        if n > 1 && !engine::part_path(&cible).exists() {
            if let Some(m) = std::fs::metadata(&cible).ok().filter(|m| m.is_file()) {
                tailles[i] = Some(m.len());
                let pleine = Live {
                    segments: vec![Segment {
                        pos: m.len(),
                        end: m.len(),
                        ..Default::default()
                    }],
                    ..Default::default()
                };
                jauge(ctl, &nom, &tailles, i, &pleine, avant);
                faits[i] = cible;
                continue;
            }
        }
        let url = p.url.as_deref().unwrap_or("");
        let googlevideo = url
            .split("://")
            .nth(1)
            .and_then(|r| r.split(['/', '?', ':']).next())
            .is_some_and(|h| h.ends_with(".googlevideo.com"));
        let opts = Options {
            connections: connexions,
            headers: p.http_headers.clone().into_iter().collect(),
            max_request: if googlevideo { PLAGE_GOOGLEVIDEO } else { 0 },
            require_ranges: true,
            ..Default::default()
        };
        // Moteur sur son propre `Control`, recopié dans la jauge unique (il remet tout à zéro) ;
        // mêmes limites de débit que l'entrée (changées en direct).
        let partie = Arc::new(Control::avec_limites(ctl.limites.clone()));
        let h = {
            let (url, dir, nom_p, c) = (
                url.to_string(),
                dir.to_path_buf(),
                nom_partie(d, i),
                partie.clone(),
            );
            std::thread::spawn(move || engine::run(&url, &dir, Some(&nom_p), &opts, &c))
        };
        loop {
            let fini = h.is_finished();
            if ctl.stop.load(Ordering::Relaxed) {
                partie.request_stop();
            }
            let l = partie.snapshot();
            tailles[i] = l.size.or(tailles[i]);
            jauge(ctl, &nom, &tailles, i, &l, avant);
            if fini {
                avant += l.received;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        match h.join().unwrap_or_else(|_| Err("panic".into()))? {
            Outcome::Done(f) => faits[i] = f,
            Outcome::Stopped => return Ok(Outcome::Stopped),
        }
    }
    if n == 1 {
        let f = faits.remove(0);
        ctl.lock().file_name = f.file_name().map(|n| n.to_string_lossy().into_owned());
        return Ok(Outcome::Done(f));
    }
    let sortie = fusion(d);
    let mut fils = commande(&ff)
        .args(["-nostdin", "-v", "error", "-y", "-i"])
        .arg(&faits[0])
        .arg("-i")
        .arg(&faits[1])
        .args(["-map", "0:v:0", "-map", "1:a:0", "-c", "copy"])
        .arg(&sortie)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "ffmpeg merge failed")?;
    // Arrêt pendant la fusion : fusion partielle effacée, parties gardées pour la reprise.
    let ok = loop {
        if ctl.stop.load(Ordering::Relaxed) {
            tuer(&mut fils);
            let _ = std::fs::remove_file(&sortie);
            return Ok(Outcome::Stopped);
        }
        match fils.try_wait() {
            Ok(Some(s)) => break s.success(),
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => {
                tuer(&mut fils);
                break false;
            }
        }
    };
    if !ok {
        return Err("ffmpeg merge failed".into());
    }
    let fini = engine::renommer_libre(&sortie, &d.fichier)?;
    for f in &faits {
        let _ = std::fs::remove_file(f);
    }
    ctl.lock().file_name = fini.file_name().map(|n| n.to_string_lossy().into_owned());
    Ok(Outcome::Done(fini))
}

/// Lignes d'une sortie de yt-dlp, octets non UTF-8 remplacés (page de code Windows) : une
/// ligne illisible n'arrête pas la lecture (sinon le tube se bloque et yt-dlp avec).
fn lignes(r: Option<impl BufRead>, mut f: impl FnMut(String) -> bool) {
    let Some(mut r) = r else { return };
    let mut b = Vec::new();
    while matches!(r.read_until(b'\n', &mut b), Ok(n) if n > 0) {
        let l = String::from_utf8_lossy(&b);
        if !f(l.trim_end_matches(['\r', '\n']).to_string()) {
            return;
        }
        b.clear();
    }
}

/// yt-dlp.exe (PyInstaller) lance un second processus : tout l'arbre est arrêté.
fn tuer(f: &mut std::process::Child) {
    #[cfg(windows)]
    {
        let _ = commande(&crate::systeme::outil("taskkill"))
            .args(["/T", "/F", "/PID", &f.id().to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = f.kill();
    let _ = f.wait();
}

/// Fichiers à télécharger (Windows) : yt-dlp.exe, ffmpeg (build partagé, plus petit) et deno.
#[derive(Clone, Debug, PartialEq)]
pub struct Paquet {
    pub ytdlp: Option<(Fichier, String)>,
    pub ffmpeg: Option<(Fichier, String)>,
    pub deno: Option<(Fichier, String)>,
}

impl Paquet {
    pub fn taille(&self) -> u64 {
        self.ytdlp
            .iter()
            .chain(&self.ffmpeg)
            .chain(&self.deno)
            .map(|f| f.0.size)
            .sum()
    }

    pub fn vide(&self) -> bool {
        self.ytdlp.is_none() && self.ffmpeg.is_none() && self.deno.is_none()
    }
}

const YTDLP_DEPOT: &str = "yt-dlp/yt-dlp";
const FFMPEG_DEPOT: &str = "yt-dlp/FFmpeg-Builds";
const FFMPEG_ZIP: &str = "ffmpeg-master-latest-win64-gpl-shared.zip";
/// Environnement JavaScript recommandé par yt-dlp (activé par défaut ; wiki « EJS »).
const DENO_DEPOT: &str = "denoland/deno";
const DENO_ZIP: &str = "deno-x86_64-pc-windows-msvc.zip";

/// Lit les releases et les empreintes de ce qui manque (rien n'est encore téléchargé).
pub fn preparer(o: &Outils) -> Result<Paquet, Erreur> {
    let lire = |depot: &str, nom: &str, sommes: &str| -> Result<(Fichier, String), Erreur> {
        let r = github::derniere(depot)?;
        let f = r.fichier(nom).ok_or("File missing from the release.")?;
        let s = r
            .fichier(sommes)
            .ok_or("Checksum missing from the release.")?;
        let sha = github::empreinte(&github::texte(&s.browser_download_url)?, nom)?;
        if !github::url_permise(&f.browser_download_url) {
            return Err("Download link refused (outside GitHub).");
        }
        Ok((f.clone(), sha))
    };
    Ok(Paquet {
        ytdlp: match o.ytdlp {
            None => Some(lire(YTDLP_DEPOT, "yt-dlp.exe", "SHA2-256SUMS")?),
            Some(_) => None,
        },
        ffmpeg: match o.ffmpeg {
            None => Some(lire(FFMPEG_DEPOT, FFMPEG_ZIP, "checksums.sha256")?),
            Some(_) => None,
        },
        deno: match o.deno {
            None => Some(lire(
                DENO_DEPOT,
                DENO_ZIP,
                &format!("{DENO_ZIP}.sha256sum"),
            )?),
            Some(_) => None,
        },
    })
}

/// Télécharge et installe le paquet dans `dossier_outils()` ; `progres(reçus, total)`.
pub fn installer(
    p: &Paquet,
    stop: &AtomicBool,
    mut progres: impl FnMut(u64, u64),
) -> Result<(), Erreur> {
    let d = dossier_outils();
    std::fs::create_dir_all(&d).map_err(|_| "Cannot write the file.")?;
    let total = p.taille();
    let mut base = 0;
    if let Some((f, sha)) = &p.ytdlp {
        github::telecharger(f, sha, &d.join("yt-dlp.exe"), stop, |n| {
            progres(base + n, total)
        })?;
        base += f.size;
    }
    if let Some((f, sha)) = &p.ffmpeg {
        let zip = d.join(FFMPEG_ZIP);
        github::telecharger(f, sha, &zip, stop, |n| progres(base + n, total))?;
        let r = extraire_ffmpeg(&zip, &d);
        let _ = std::fs::remove_file(&zip);
        r?;
        base += f.size;
    }
    if let Some((f, sha)) = &p.deno {
        let zip = d.join(DENO_ZIP);
        github::telecharger(f, sha, &zip, stop, |n| progres(base + n, total))?;
        let r = extraire_deno(&zip, &d);
        let _ = std::fs::remove_file(&zip);
        r?;
    }
    Ok(())
}

/// Zip extrait par tar.exe (Windows 10 et plus) dans le dossier `tmp` (recréé vide).
fn tar_x(zip: &Path, tmp: &Path) -> Result<bool, Erreur> {
    let _ = std::fs::remove_dir_all(tmp);
    std::fs::create_dir_all(tmp).map_err(|_| "Cannot write the file.")?;
    // Chemin complet : pas de tar.exe pris dans le dossier courant ou le PATH.
    let tar = crate::systeme::outil("tar");
    Ok(commande(&tar)
        .arg("-xf")
        .arg(zip)
        .arg("-C")
        .arg(tmp)
        .status()
        .is_ok_and(|s| s.success()))
}

/// Le zip contient `deno.exe` seul : extrait dans un dossier temporaire, puis déplacé.
fn extraire_deno(zip: &Path, d: &Path) -> Result<(), Erreur> {
    let tmp = d.join("deno.tmp");
    let exe = tmp.join("deno.exe");
    let r = if tar_x(zip, &tmp)? && exe.is_file() {
        std::fs::rename(&exe, d.join("deno.exe")).map_err(|_| "Cannot write the file.")
    } else {
        Err("Cannot unpack deno.")
    };
    let _ = std::fs::remove_dir_all(&tmp);
    r
}

/// Le zip contient `ffmpeg-…-shared/bin/*` : extrait dans un dossier temporaire, puis renommé
/// en `ffmpeg/`.
fn extraire_ffmpeg(zip: &Path, d: &Path) -> Result<(), Erreur> {
    let tmp = d.join("ffmpeg.tmp");
    let ok = tar_x(zip, &tmp)?;
    let interne = std::fs::read_dir(&tmp)
        .ok()
        .and_then(|mut l| l.find_map(|e| e.ok().map(|e| e.path())))
        .filter(|p| p.join("bin").join("ffmpeg.exe").is_file());
    let r = match (ok, interne) {
        (true, Some(p)) => {
            let fin = d.join("ffmpeg");
            let _ = std::fs::remove_dir_all(&fin);
            std::fs::rename(p, fin).map_err(|_| "Cannot write the file.")
        }
        _ => Err("Cannot unpack ffmpeg."),
    };
    let _ = std::fs::remove_dir_all(&tmp);
    r
}

/// `yt-dlp -U` sur notre copie (pas celle du système), dans un fil.
pub fn mettre_a_jour_ytdlp() {
    let yt = dossier_outils().join(format!("yt-dlp{EXE}"));
    if yt.is_file() {
        std::thread::spawn(move || {
            let _ = commande(&yt)
                .arg("-U")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Audit L19 : un octet non UTF-8 (page de code Windows) n'arrête pas la lecture.
    #[test]
    fn sortie_non_utf8_lue_jusqu_au_bout() {
        let brut: &[u8] = b"[download] Destination: caf\xe9.mp4\r\nfluxdl 10 100 NA\nfin";
        let mut l = Vec::new();
        lignes(Some(brut), |x| {
            l.push(x);
            true
        });
        assert_eq!(
            l,
            [
                "[download] Destination: caf\u{fffd}.mp4",
                "fluxdl 10 100 NA",
                "fin"
            ]
        );
    }

    /// Audit H1 : jamais de fichier de config ni de greffon lu (un `yt-dlp.conf` déposé dans
    /// Téléchargements contenait `--exec`) ; dossier courant sûr.
    #[test]
    fn sans_config_ni_greffon() {
        let notre = dossier_outils().join(format!("yt-dlp{EXE}"));
        let deno = dossier_outils().join(format!("deno{EXE}"));
        for (yt, greffons) in [(notre, true), (PathBuf::from("/usr/bin/yt-dlp"), false)] {
            let o = Outils {
                ytdlp: Some(yt.clone()),
                ffmpeg: None,
                deno: greffons.then(|| deno.clone()),
            };
            let mut c = commande(&yt);
            args_communs(&mut c, &o, None);
            let a: Vec<_> = c
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            assert_eq!(a.first().map(String::as_str), Some("--ignore-config"));
            assert_eq!(a.iter().any(|x| x == "--no-plugin-dirs"), greffons);
            // Notre deno par son chemin complet (YouTube exige un environnement JavaScript)
            let js = if greffons {
                format!("deno:{}", deno.display())
            } else {
                "deno".into()
            };
            assert!(
                a.windows(2).any(|w| w[0] == "--js-runtimes" && w[1] == js),
                "{a:?}"
            );
            assert_eq!(
                c.get_current_dir(),
                Some(crate::store::data_dir().as_path())
            );
        }
    }

    /// Sortie réelle de `yt-dlp -J` (Wikimedia Commons, Big Buck Bunny 4K, 2026-11-26) :
    /// formats image + son ensemble, une source sans hauteur, une taille négative (mpd).
    const COMMONS: &[u8] = include_bytes!("../tests/fixtures/ytdlp_commons.json");

    #[test]
    fn formats_d_une_vraie_sortie() {
        let i = infos_depuis_json(COMMONS).unwrap();
        assert_eq!(i.titre, "Big Buck Bunny 4K");
        assert_eq!(i.hauteur_max, 2250);
        assert_eq!(nom_hauteur(i.hauteur_max), "4K");
        let c = |q| i.choix.iter().find(|c| c.0 == q).copied().unwrap();
        // Meilleure : source 2250 px (taille connue) ; 1080p : le webm 1080 (son compris).
        assert_eq!(c(Qualite::Best), (Qualite::Best, true, Some(2_964_839_055)));
        assert_eq!(c(Qualite::P1080), (Qualite::P1080, true, Some(300_460_845)));
        assert_eq!(c(Qualite::P720), (Qualite::P720, true, Some(96_610_694)));
        // Pas de son seul : extraction possible (son dans les vidéos), taille inconnue.
        assert_eq!(c(Qualite::Mp3), (Qualite::Mp3, true, None));
    }

    #[test]
    fn formats_separes_a_la_youtube() {
        // Synthétique, au format de YouTube : images et sons séparés, 8K.
        let j = br#"{"title":"T","duration":61.0,"formats":[
            {"format_id":"140","ext":"m4a","vcodec":"none","acodec":"mp4a.40.2","tbr":129.5,"filesize":1000000},
            {"format_id":"251","ext":"webm","vcodec":"none","acodec":"opus","tbr":140.1,"filesize":1100000},
            {"format_id":"137","ext":"mp4","height":1080,"vcodec":"avc1","acodec":"none","tbr":4000,"filesize":30000000},
            {"format_id":"248","ext":"webm","height":1080,"vcodec":"vp9","acodec":"none","tbr":2600,"filesize_approx":20000000},
            {"format_id":"136","ext":"mp4","height":720,"vcodec":"avc1","acodec":"none","tbr":2000,"filesize":15000000},
            {"format_id":"571","ext":"mp4","height":4320,"vcodec":"av01","acodec":"none","tbr":30000,"filesize":200000000},
            {"format_id":"18","ext":"mp4","height":360,"vcodec":"avc1","acodec":"mp4a","tbr":500,"filesize":4000000}]}"#;
        let i = infos_depuis_json(j).unwrap();
        assert_eq!(
            (i.hauteur_max, nom_hauteur(i.hauteur_max).as_str()),
            (4320, "8K")
        );
        let c = |q| i.choix.iter().find(|c| c.0 == q).unwrap().2;
        assert_eq!(c(Qualite::Best), Some(201_100_000));
        assert_eq!(c(Qualite::P1080), Some(31_100_000));
        assert_eq!(c(Qualite::P720), Some(16_100_000));
        // Son original : le meilleur (Opus ici), sans conversion.
        assert_eq!(c(Qualite::M4a), Some(1_100_000));
        assert_eq!(c(Qualite::Mp3), Some(1_100_000));
        // Vidéo en 480p : 1080p et 720p grisés.
        let petit = br#"{"title":"P","formats":[{"height":480,"vcodec":"avc1","acodec":"mp4a","filesize":5}]}"#;
        let i = infos_depuis_json(petit).unwrap();
        assert!(!i.choix[1].1 && !i.choix[2].1 && i.choix[0].1);
        assert!(infos_depuis_json(b"<html>").is_err());
    }

    #[test]
    fn passage_live_et_cookies_dans_les_arguments() {
        assert_eq!(temps("1:02:03"), Some(3723));
        assert_eq!(temps(" 2:03 "), Some(123));
        assert_eq!(temps("45"), Some(45));
        assert_eq!(temps("90:00"), Some(5400));
        for x in ["", "1:60", "a", "1::2", "1:2:3:4", "-1", "1:2.5"] {
            assert_eq!(temps(x), None, "{x}");
        }
        assert_eq!(hms(3723), "01:02:03");
        let t = Tache {
            qualite: Qualite::P720,
            estimation: None,
            section: Some((60, 3723)),
            debut: false,
            sortie: Sortie::default(),
        };
        let a = t.args();
        let apres = |o: &str| a.iter().position(|x| x == o).map(|i| a[i + 1].as_str());
        assert_eq!(apres("--download-sections"), Some("*00:01:00-01:02:03"));
        assert_eq!(
            apres("-o"),
            Some("%(title).150B [%(id)s] 00.01.00-01.02.03.%(ext)s")
        );
        assert!(!a.iter().any(|x| x == "--live-from-start"));
        let live = Tache {
            debut: true,
            section: None,
            ..t
        };
        let a = live.args();
        assert!(a.iter().any(|x| x == "--live-from-start"));
        assert!(!a.iter().any(|x| x == "--download-sections"));
        assert!(debut_possible("https://www.youtube.com/live/abc"));
        assert!(!debut_possible("https://www.twitch.tv/chaine"));
        // --cookies seulement avec un fichier, toujours après --ignore-config.
        let o = Outils::default();
        let mut c = commande(Path::new("/usr/bin/yt-dlp"));
        args_communs(&mut c, &o, Some(Path::new("/tmp/c.txt")));
        let a: Vec<_> = c
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(a[0], "--ignore-config");
        let i = a.iter().position(|x| x == "--cookies").unwrap();
        assert_eq!(a[i + 1], "/tmp/c.txt");
        let mut c = commande(Path::new("/usr/bin/yt-dlp"));
        args_communs(&mut c, &o, None);
        assert!(!c.get_args().any(|a| a == "--cookies"));
    }

    #[test]
    fn liens_directs_lus() {
        let dir = Path::new("/dl");
        let j = r#"{"requested_formats":[
            {"url":"https://r1.googlevideo.com/v?sig=1","protocol":"https","http_headers":{"User-Agent":"UA"}},
            {"url":"https://r1.googlevideo.com/a","protocol":"https"}],"url":null}"#;
        let j = j.replace('\n', "");
        let d = directs(&format!("../../etc/T [x].mp4\n{j}\n"), dir).unwrap();
        // Nom seulement : jamais hors du dossier de téléchargement.
        assert_eq!(d.fichier, Path::new("/dl/T [x].mp4"));
        assert_eq!(d.parties.len(), 2);
        assert_eq!(d.parties[0].http_headers["User-Agent"], "UA");
        assert_eq!(nom_partie(&d, 1), "T [x].mp4.fluxdl1");
        assert_eq!(fusion(&d), Path::new("/dl/T [x].fluxdl.mp4"));
        let seul = r#"{"url":"http://h/v.mp4","protocol":"http"}"#;
        let d = directs(&format!("T.webm\n{seul}"), dir).unwrap();
        assert_eq!((d.parties.len(), nom_partie(&d, 0).as_str()), (1, "T.webm"));
        // Fragments (HLS de Twitch, DASH), live, autre schéma : yt-dlp seul.
        for j in [
            r#"{"url":"https://h/i.m3u8","protocol":"m3u8_native"}"#,
            r#"{"url":"https://h/v.mp4","protocol":"https","is_live":true}"#,
            r#"{"url":"file:///etc/passwd","protocol":"https"}"#,
            r#"{"requested_formats":[{"url":"https://h/v","protocol":"https"},{"url":"https://h/a","protocol":"http_dash_segments"}]}"#,
        ] {
            assert_eq!(directs(&format!("T.mp4\n{j}"), dir), None, "{j}");
        }
        assert_eq!(directs("T.mp4\n<html>", dir), None);
        assert_eq!(directs("", dir), None);
    }

    /// Serveur local : `(chemin, octets)` ; `plages` faux = Range ignoré (200 entier, comme un
    /// CDN qui ne découpe pas).
    #[cfg(unix)]
    fn serveur(fichiers: Vec<(String, Vec<u8>)>, plages: bool) -> String {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let fichiers = Arc::new(fichiers);
        std::thread::spawn(move || {
            for mut s in l.incoming().flatten() {
                let f = fichiers.clone();
                std::thread::spawn(move || {
                    let mut r = BufReader::new(s.try_clone()?);
                    let mut tete = String::new();
                    while r.read_line(&mut tete)? > 2 && !tete.ends_with("\r\n\r\n") {}
                    let chemin = tete.split_whitespace().nth(1).unwrap_or("").to_string();
                    let plage = tete
                        .lines()
                        .find_map(|h| {
                            h.split_once(':')
                                .filter(|(k, _)| k.eq_ignore_ascii_case("range"))
                        })
                        .and_then(|(_, v)| v.trim().strip_prefix("bytes=")?.split_once('-'))
                        .and_then(|(a, b)| {
                            Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?))
                        });
                    let Some((_, d)) = f.iter().find(|(c, _)| *c == chemin) else {
                        return s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    };
                    let (statut, corps) = match plage.filter(|_| plages) {
                        Some((a, b)) => {
                            let b = b.min(d.len() - 1);
                            (format!("206 Partial Content\r\nAccept-Ranges: bytes\r\nContent-Range: bytes {a}-{b}/{}", d.len()), &d[a..=b])
                        }
                        None => ("200 OK".to_string(), &d[..]),
                    };
                    write!(s, "HTTP/1.1 {statut}\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", corps.len())?;
                    s.write_all(corps)?;
                    let _ = r.read(&mut [0; 1]);
                    Ok::<_, std::io::Error>(())
                });
            }
        });
        base
    }

    /// 0.2.8 : faux yt-dlp (script) qui donne des liens directs vers un serveur local ; les
    /// parties passent par le moteur multi-connexions puis ffmpeg (s'il est là) ; serveur qui
    /// refuse les plages → yt-dlp seul (fragments en parallèle), sans erreur ni fichier de travail.
    #[cfg(unix)]
    #[test]
    fn video_par_le_moteur_et_repli() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("fluxdl-test-video-{}", std::process::id()));
        let dl = d.join("dl");
        std::fs::create_dir_all(&dl).unwrap();
        let yt = d.join("yt-dlp");
        std::fs::write(
            &yt,
            format!(
                "#!/bin/sh\necho \"$@\" >> '{0}/args.txt'\ncase \"$*\" in\n*\"--print filename\"*) cat '{0}/sortie.txt' ;;\n*) printf yt > '{1}/ytdlp.mp4'; echo 'fluxdl-fichier {1}/ytdlp.mp4' ;;\nesac\n",
                d.display(),
                dl.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&yt, std::fs::Permissions::from_mode(0o755)).unwrap();
        let o = Outils {
            ytdlp: Some(yt),
            ..trouver()
        };
        let tache = Tache {
            qualite: Qualite::Best,
            estimation: None,
            section: None,
            debut: false,
            sortie: Sortie::default(),
        };
        let lance = |sortie: String| {
            std::fs::write(d.join("sortie.txt"), sortie).unwrap();
            // Limites de l'entrée (globale illimitée, propre 50 Mo/s) : `--limit-rate` à yt-dlp.
            let ctl = Arc::new(Control::avec_limites(vec![
                Arc::new(engine::Seau::new(0)),
                Arc::new(engine::Seau::new(50_000_000)),
            ]));
            let r = telecharger(
                &o,
                "https://www.youtube.com/watch?v=x",
                &dl,
                &tache,
                None,
                4,
                &ctl,
            );
            (r.unwrap(), ctl.snapshot())
        };
        let reste = || {
            let mut v: Vec<String> = std::fs::read_dir(&dl)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        };
        // Un seul lien direct (3 Mio) : découpé sur plusieurs connexions, écrit sous son nom.
        let gros: Vec<u8> = (0..3u32 << 20).map(|i| (i * 7 % 251) as u8).collect();
        let base = serveur(vec![("/v".into(), gros.clone())], true);
        let j = format!(r#"{{"url":"{base}/v","protocol":"http","http_headers":{{"X-T":"1"}}}}"#);
        let (r, l) = lance(format!("{}/S [x].mp4\n{j}\n", dl.display()));
        assert_eq!(r, Outcome::Done(dl.join("S [x].mp4")));
        assert_eq!(std::fs::read(dl.join("S [x].mp4")).unwrap(), gros);
        assert!(l.ranges && l.segments.len() >= 2, "{l:?}");
        assert_eq!(reste(), ["S [x].mp4"]);
        // Image + son (ffmpeg du PATH) : fusion sans réencodage, fichiers de travail effacés.
        if let Some(ff) = &o.ffmpeg {
            let ff = ff.join(format!("ffmpeg{EXE}"));
            let gen = |args: &str, f: &str| {
                let ok = Command::new(&ff)
                    .args(["-nostdin", "-v", "error", "-y", "-f", "lavfi"])
                    .args(args.split(' '))
                    .arg(d.join(f))
                    .status()
                    .unwrap();
                assert!(ok.success());
                std::fs::read(d.join(f)).unwrap()
            };
            let v = gen(
                "-i testsrc=s=160x120:d=2 -c:v libx264 -pix_fmt yuv420p",
                "v.mp4",
            );
            let a = gen("-i sine=d=2 -c:a aac", "a.m4a");
            let base = serveur(vec![("/v".into(), v), ("/a".into(), a)], true);
            let j = format!(
                r#"{{"requested_formats":[{{"url":"{base}/v","protocol":"https"}},{{"url":"{base}/a","protocol":"https"}}]}}"#
            );
            let (r, _) = lance(format!("{}/F [x].mp4\n{j}\n", dl.display()));
            assert_eq!(r, Outcome::Done(dl.join("F [x].mp4")));
            let info = Command::new(&ff)
                .arg("-i")
                .arg(dl.join("F [x].mp4"))
                .output()
                .unwrap();
            let info = String::from_utf8_lossy(&info.stderr);
            assert!(
                info.contains("Video: h264") && info.contains("Audio: aac"),
                "{info}"
            );
            assert_eq!(reste(), ["F [x].mp4", "S [x].mp4"]);
        } else {
            eprintln!("ffmpeg absent : fusion non testée");
        }
        // Plages refusées : repli sur yt-dlp seul, fragments en parallèle.
        let base = serveur(vec![("/v".into(), gros)], false);
        let j = format!(r#"{{"url":"{base}/v","protocol":"https"}}"#);
        let (r, _) = lance(format!("{}/R [x].mp4\n{j}\n", dl.display()));
        assert_eq!(r, Outcome::Done(dl.join("ytdlp.mp4")));
        assert!(
            !reste().iter().any(|f| f.starts_with("R [x]")),
            "{:?}",
            reste()
        );
        let args = std::fs::read_to_string(d.join("args.txt")).unwrap();
        assert!(args.contains("--concurrent-fragments 4"), "{args}");
        assert!(args.contains("--limit-rate 50000000"), "{args}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 0.2.9 : une seule jauge pour le son puis l'image, qui ne repart pas de zéro.
    #[test]
    fn jauge_unique_sur_les_parties() {
        let ctl = Control::default();
        let tailles = [Some(200), Some(100)];
        let seg = |pos, end| Segment {
            start: 0,
            pos,
            end,
            worker: Some(0),
            stolen: false,
        };
        let en = |segments, received| Live {
            segments,
            received,
            workers: vec![WorkerStat::default()],
            ..Default::default()
        };
        let etapes = [
            (1, Live::default(), 0),
            (1, en(vec![seg(50, 100)], 50), 0),
            (1, en(vec![seg(100, 100)], 100), 0),
            (0, Live::default(), 100),
            (0, en(vec![seg(80, 200)], 80), 100),
        ];
        let mut faits = Vec::new();
        for (i, l, avant) in &etapes {
            jauge(&ctl, "F.mp4", &tailles, *i, l, *avant);
            let v = ctl.snapshot();
            assert_eq!((v.size, v.file_name.as_deref()), (Some(300), Some("F.mp4")));
            faits.push((v.done(), v.received));
        }
        assert_eq!(
            faits,
            [(0, 0), (50, 50), (100, 100), (100, 100), (180, 180)]
        );
        // Image en cours : décalée après le son, connexion gardée.
        let v = ctl.snapshot();
        assert_eq!(
            v.segments[1],
            Segment {
                start: 100,
                pos: 180,
                end: 300,
                ..seg(0, 0)
            }
        );
        assert_eq!(v.workers.len(), 1);
        jauge(&ctl, "F.mp4", &[None, Some(100)], 1, &Live::default(), 0);
        assert_eq!(ctl.snapshot().size, None);
    }

    /// 0.2.9 : image + son par le moteur : jauge cumulée monotone, nom final affiché ; arrêt
    /// pendant la fusion (parties gardées) puis reprise sans retélécharger ; arrêt pendant
    /// `--print`.
    #[cfg(unix)]
    #[test]
    fn video_jauge_nom_arret_reprise() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::Instant;
        let d = std::env::temp_dir().join(format!("fluxdl-test-video2-{}", std::process::id()));
        let (dl, ff) = (d.join("dl"), d.join("ff"));
        std::fs::create_dir_all(&dl).unwrap();
        std::fs::create_dir_all(&ff).unwrap();
        let script = |f: PathBuf, corps: String| {
            std::fs::write(&f, format!("#!/bin/sh\n{corps}")).unwrap();
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
            f
        };
        let yt = script(
            d.join("yt-dlp"),
            format!(
                "case \"$*\" in\n*\"--print filename\"*) [ -e '{0}/dort' ] && exec sleep 30; cat '{0}/sortie.txt' ;;\n*) printf yt > '{1}/ytdlp.mp4'; echo 'fluxdl-fichier {1}/ytdlp.mp4' ;;\nesac\n",
                d.display(),
                dl.display()
            ),
        );
        // Faux ffmpeg : écrit la fusion (dernier argument), puis dort si demandé.
        script(
            ff.join("ffmpeg"),
            format!(
                "for a; do o=$a; done\nprintf fusion > \"$o\"\n[ -e '{}/ff_dort' ] && exec sleep 30\nexit 0\n",
                d.display()
            ),
        );
        let o = Outils {
            ytdlp: Some(yt),
            ffmpeg: Some(ff),
            ..Default::default()
        };
        let lance = |ctl: &Arc<Control>| {
            let (o, dl, ctl) = (o.clone(), dl.clone(), ctl.clone());
            std::thread::spawn(move || {
                let tache = Tache {
                    qualite: Qualite::Best,
                    estimation: None,
                    section: None,
                    debut: false,
                    sortie: Sortie::default(),
                };
                telecharger(
                    &o,
                    "https://www.youtube.com/watch?v=x",
                    &dl,
                    &tache,
                    None,
                    4,
                    &ctl,
                )
            })
        };
        let v: Vec<u8> = (0..2u32 << 20).map(|i| (i * 7 % 251) as u8).collect();
        let a: Vec<u8> = (0..1u32 << 20).map(|i| (i * 3 % 241) as u8).collect();
        let total = (v.len() + a.len()) as u64;
        let (lv, la) = (v.len(), a.len());
        let base = serveur(vec![("/v".into(), v), ("/a".into(), a)], true);
        let sortie = |base: &str| {
            let j = format!(
                r#"{{"requested_formats":[{{"url":"{base}/v","protocol":"https","filesize":{lv}}},{{"url":"{base}/a","protocol":"https","filesize_approx":{la}}}]}}"#
            );
            std::fs::write(
                d.join("sortie.txt"),
                format!("{}/G.mp4\n{j}\n", dl.display()),
            )
            .unwrap();
        };
        sortie(&base);
        std::fs::write(d.join("ff_dort"), "").unwrap();
        let ctl = Arc::new(Control::default());
        let h = lance(&ctl);
        let fusion = dl.join("G.fluxdl.mp4");
        let t0 = Instant::now();
        let mut vus = Vec::new();
        while !fusion.exists() {
            assert!(!h.is_finished() && t0.elapsed() < Duration::from_secs(20));
            let l = ctl.snapshot();
            vus.push((l.done(), l.size, l.file_name));
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(vus.windows(2).all(|w| w[0].0 <= w[1].0), "{vus:?}");
        for (_, size, nom) in &vus {
            if nom.is_some() {
                assert_eq!((nom.as_deref(), *size), (Some("G.mp4"), Some(total)));
            }
        }
        let l = ctl.snapshot();
        assert_eq!((l.done(), l.file_name.as_deref()), (total, Some("G.mp4")));
        // Arrêt pendant la fusion : rendu vite, fusion partielle effacée, parties gardées.
        ctl.request_stop();
        let t = Instant::now();
        assert_eq!(h.join().unwrap(), Ok(Outcome::Stopped));
        assert!(t.elapsed() < Duration::from_secs(2));
        assert!(!fusion.exists());
        let taille = |n: &str| std::fs::metadata(dl.join(n)).map(|m| m.len() as usize).ok();
        assert_eq!(
            (taille("G.mp4.fluxdl0"), taille("G.mp4.fluxdl1")),
            (Some(lv), Some(la))
        );
        // Reprise : liens morts (404), les parties finies ne sont pas redemandées.
        std::fs::remove_file(d.join("ff_dort")).unwrap();
        sortie(&format!("{base}/absent"));
        let ctl = Arc::new(Control::default());
        assert_eq!(
            lance(&ctl).join().unwrap(),
            Ok(Outcome::Done(dl.join("G.mp4")))
        );
        assert_eq!(std::fs::read(dl.join("G.mp4")).unwrap(), b"fusion");
        let l = ctl.snapshot();
        assert_eq!((l.done(), l.size), (total, Some(total)));
        assert!(taille("G.mp4.fluxdl0").is_none() && taille("G.mp4.fluxdl1").is_none());
        // Arrêt pendant `--print` (yt-dlp qui dort) : yt-dlp tué, rendu vite.
        std::fs::write(d.join("dort"), "").unwrap();
        let ctl = Arc::new(Control::default());
        let h = lance(&ctl);
        std::thread::sleep(Duration::from_millis(300));
        ctl.request_stop();
        let t = Instant::now();
        assert_eq!(h.join().unwrap(), Ok(Outcome::Stopped));
        assert!(t.elapsed() < Duration::from_secs(2));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn lignes_de_progression_et_sites() {
        assert_eq!(progression("fluxdl 1024 2048 NA"), Some((1024, Some(2048))));
        assert_eq!(
            progression("fluxdl 1024 NA 4096.5"),
            Some((1024, Some(4096)))
        );
        assert_eq!(progression("fluxdl 10 NA NA"), Some((10, None)));
        assert_eq!(progression("[download] 5%"), None);
        assert!(site_video("https://www.youtube.com/watch?v=x"));
        assert!(site_video("https://youtu.be/x"));
        assert!(site_video("https://m.youtube.com/watch?v=x"));
        assert!(!site_video("https://notyoutube.com/x"));
        assert!(!site_video("https://example.org/youtube.com"));
        assert_eq!(
            derniere_erreur("WARNING: a\nERROR: [youtube] x: Video unavailable\n"),
            "[youtube] x: Video unavailable"
        );
    }

    /// 0.2.9 : arguments de yt-dlp pour chaque combinaison de sous-titres, miniature,
    /// chapitres et conteneur, en vidéo et en son seul.
    #[test]
    fn arguments_de_sortie() {
        let a = |qualite, sortie| {
            Tache {
                qualite,
                estimation: None,
                section: None,
                debut: false,
                sortie,
            }
            .args()
            .join(" ")
        };
        // Sous-titres : (case, langue, auto, srt) → ce qui doit être passé (ou rien).
        let st = [
            (false, Some("fr"), false, false, None),
            (true, None, false, false, None),
            (
                true,
                Some("fr"),
                false,
                false,
                Some("--embed-subs --sub-langs fr"),
            ),
            (
                true,
                Some("fr"),
                false,
                true,
                Some("--write-subs --sub-langs fr --convert-subs srt"),
            ),
            (
                true,
                Some("en-US"),
                true,
                false,
                Some("--write-auto-subs --embed-subs --sub-langs en-US"),
            ),
            (
                true,
                Some("en-US"),
                true,
                true,
                Some("--write-auto-subs --sub-langs en-US --convert-subs srt"),
            ),
        ];
        let mut n = 0;
        for q in Qualite::TOUTES {
            for &(subtitles, l, sub_auto, srt, attendu) in &st {
                for (thumbnail, chapters, mp4) in (0..8).map(|b| (b & 1 > 0, b & 2 > 0, b & 4 > 0))
                {
                    let s = Sortie {
                        subtitles,
                        sub_lang: l.map(String::from),
                        sub_auto,
                        srt,
                        thumbnail,
                        chapters,
                        mp4,
                    };
                    let x = a(q, s);
                    let ctx = format!("{q:?} {x}");
                    assert!(x.starts_with(&q.args().join(" ")), "{ctx}");
                    assert!(x.ends_with("-o %(title).150B [%(id)s].%(ext)s"), "{ctx}");
                    assert_eq!(x.contains("--embed-thumbnail"), thumbnail, "{ctx}");
                    assert_eq!(x.contains("--embed-chapters"), chapters, "{ctx}");
                    // Son seul : ni sous-titres ni conteneur vidéo.
                    let image = !q.audio();
                    assert_eq!(
                        x.contains("-S vcodec:h264,acodec:m4a --merge-output-format mp4"),
                        mp4 && image,
                        "{ctx}"
                    );
                    assert_eq!(x.contains("--merge-output-format"), mp4 && image, "{ctx}");
                    match attendu.filter(|_| image) {
                        Some(v) => assert!(x.contains(v), "{ctx}"),
                        None => assert!(!x.contains("subs"), "{ctx}"),
                    }
                    // Intégrés : jamais --write-subs (le fichier resterait à côté).
                    if !srt {
                        assert!(!x.contains("--write-subs"), "{ctx}");
                    }
                    n += 1;
                }
            }
        }
        assert_eq!(n, 5 * 6 * 8);
        // Son : MP3 converti, original sans conversion.
        assert_eq!(
            a(Qualite::Mp3, Sortie::default()),
            "-f ba/b -x --audio-format mp3 --audio-quality 0 -o %(title).150B [%(id)s].%(ext)s"
        );
        assert_eq!(
            a(Qualite::M4a, Sortie::default()),
            "-f ba/b -x -o %(title).150B [%(id)s].%(ext)s"
        );
        // Anciennes entrées de queue.json (sans `options`) : rien d'ajouté, conteneur original.
        let k: crate::store::Kind =
            serde_json::from_str(r#"{"Video":{"quality":"P1080","estimate":null}}"#).unwrap();
        assert!(
            matches!(k, crate::store::Kind::Video { options, .. } if options == Sortie::default())
        );
        let s = Sortie {
            subtitles: true,
            sub_lang: Some("fr".into()),
            mp4: true,
            ..Default::default()
        };
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<Sortie>(&j).unwrap(), s);
    }

    /// 0.2.9 : langues de sous-titres lues dans `-J` (site puis automatiques), langue de
    /// l'interface par défaut.
    #[test]
    fn langues_de_sous_titres() {
        let j = br#"{"title":"T","formats":[],
            "subtitles":{"fr-CA":[{"ext":"vtt","name":"French (Canada)"}],"fr":[{"ext":"vtt","url":"u","name":"French"}],
                "live_chat":[{"ext":"json"}],"x(.*)":[{"name":"pirate"}],"de":[{"ext":"srv3"}]},
            "automatic_captions":{"en":[{"name":"English"}],"en-orig":[{"name":"English (Original)"}],"fr":[{"name":"French"}]}}"#;
        let i = infos_depuis_json(j).unwrap();
        let v: Vec<_> = i
            .sous_titres
            .iter()
            .map(|s| (s.code.as_str(), s.nom.as_str(), s.auto))
            .collect();
        assert_eq!(
            v,
            [
                ("de", "de", false),
                ("fr", "French", false),
                ("fr-CA", "French (Canada)", false),
                ("en", "English", true),
                ("en-orig", "English (Original)", true),
                ("fr", "French", true),
            ]
        );
        let d = |l| sous_titres_par_defaut(&i.sous_titres, l).map(|s| (s.code.as_str(), s.auto));
        // Ceux du site avant les automatiques, code exact avant les variantes.
        assert_eq!(d("fr"), Some(("fr", false)));
        assert_eq!(d("en"), Some(("en", true)));
        assert_eq!(d("es"), None);
        let seul = infos_depuis_json(br#"{"automatic_captions":{"en-US":[{}]}}"#).unwrap();
        assert_eq!(
            sous_titres_par_defaut(&seul.sous_titres, "en").map(|s| s.code.as_str()),
            Some("en-US")
        );
        // Vraie sortie (Commons) : 31 langues du site, sans nom (code affiché), fr choisi.
        let c = infos_depuis_json(COMMONS).unwrap().sous_titres;
        assert_eq!(c.len(), 31);
        assert!(c.iter().all(|s| !s.auto && s.nom == s.code));
        assert_eq!(
            sous_titres_par_defaut(&c, "zh").map(|s| s.code.as_str()),
            Some("zh-Hans-CN")
        );
        assert_eq!(
            sous_titres_par_defaut(&c, "fr").map(|s| s.code.as_str()),
            Some("fr")
        );
        assert!(infos_depuis_json(br#"{"formats":[]}"#)
            .unwrap()
            .sous_titres
            .is_empty());
    }

    /// 0.2.9 : sous-titres, miniature, chapitres avec le passage par le moteur : fichier fini
    /// par le moteur, puis yt-dlp seul le retrouve et y fait les ajouts (passe finale) ;
    /// fichier déjà là = passe seule ; webm + miniature → mkv ; passe ratée = vidéo gardée ;
    /// sans ffmpeg = pas d'ajouts.
    #[cfg(unix)]
    #[test]
    fn video_ajouts_apres_le_moteur() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("fluxdl-test-video3-{}", std::process::id()));
        let (dl, ff) = (d.join("dl"), d.join("ff"));
        std::fs::create_dir_all(&dl).unwrap();
        std::fs::create_dir_all(&ff).unwrap();
        let script = |f: PathBuf, corps: String| {
            std::fs::write(&f, format!("#!/bin/sh\n{corps}")).unwrap();
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
            f
        };
        // Faux yt-dlp : `--print` = sortie.txt ; sinon passe finale (échoue si `rate`), qui
        // donne le fichier retrouvé (final.txt) sans rien écrire.
        let yt = script(
            d.join("yt-dlp"),
            format!(
                "echo \"$@\" >> '{0}/args.txt'\ncase \"$*\" in\n*\"--print filename\"*) cat '{0}/sortie.txt' ;;\n*) [ -e '{0}/rate' ] && {{ echo 'ERROR: Postprocessing: boom' >&2; exit 1; }}; echo \"fluxdl-fichier $(cat '{0}/final.txt')\" ;;\nesac\n",
                d.display()
            ),
        );
        script(
            ff.join("ffmpeg"),
            "for a; do o=$a; done\nprintf fusion > \"$o\"\nexit 0\n".into(),
        );
        let o = Outils {
            ytdlp: Some(yt),
            ffmpeg: Some(ff),
            ..Default::default()
        };
        let tache = Tache {
            qualite: Qualite::Best,
            estimation: None,
            section: None,
            debut: false,
            sortie: Sortie {
                subtitles: true,
                sub_lang: Some("fr".into()),
                thumbnail: true,
                chapters: true,
                mp4: true,
                ..Default::default()
            },
        };
        let lance = |o: &Outils, sortie: String, final_: &str| {
            std::fs::write(d.join("sortie.txt"), sortie).unwrap();
            std::fs::write(d.join("final.txt"), dl.join(final_).display().to_string()).unwrap();
            let _ = std::fs::remove_file(d.join("args.txt"));
            let ctl = Arc::new(Control::default());
            let r = telecharger(
                o,
                "https://www.youtube.com/watch?v=x",
                &dl,
                &tache,
                None,
                4,
                &ctl,
            );
            let args = std::fs::read_to_string(d.join("args.txt")).unwrap();
            (r, args.lines().map(String::from).collect::<Vec<_>>())
        };
        let v: Vec<u8> = (0..2u32 << 20).map(|i| (i * 7 % 251) as u8).collect();
        let base = serveur(
            vec![("/v".into(), v.clone()), ("/a".into(), vec![1; 1000])],
            true,
        );
        let un = format!(r#"{{"url":"{base}/v","protocol":"https"}}"#);
        let sortie = |nom: &str, j: &str| format!("{}/{nom}\n{j}\n", dl.display());
        // Lien direct : le moteur télécharge, puis la passe finale (mêmes arguments) ajoute.
        let (r, args) = lance(&o, sortie("E [x].mp4", &un), "E [x].mp4");
        assert_eq!(r, Ok(Outcome::Done(dl.join("E [x].mp4"))));
        assert_eq!(std::fs::read(dl.join("E [x].mp4")).unwrap(), v);
        assert_eq!(args.len(), 2, "{args:?}");
        assert!(args[0].contains("--print filename"));
        for a in &args {
            for x in [
                "-S vcodec:h264,acodec:m4a --merge-output-format mp4",
                "--embed-subs --sub-langs fr",
                "--embed-thumbnail",
                "--embed-chapters",
            ] {
                assert!(a.contains(x), "{x} : {a}");
            }
        }
        assert!(args[1].contains("--no-simulate") && !args[1].contains("--print filename"));
        // Déjà là (passe arrêtée avant la fin) : pas de nouveau téléchargement, passe seule.
        let (r, args) = lance(&o, sortie("E [x].mp4", &un), "E [x].mp4");
        assert_eq!(r, Ok(Outcome::Done(dl.join("E [x].mp4"))));
        assert_eq!(args.len(), 2);
        assert!(!dl.join("E [x] (1).mp4").exists());
        // Image + son en webm avec miniature : fusion en mkv, nom retrouvé par yt-dlp.
        let deux = format!(
            r#"{{"requested_formats":[{{"url":"{base}/v","protocol":"https"}},{{"url":"{base}/a","protocol":"https"}}]}}"#
        );
        let (r, _) = lance(&o, sortie("W [x].webm", &deux), "W [x].mkv");
        assert_eq!(r, Ok(Outcome::Done(dl.join("W [x].mkv"))));
        assert_eq!(std::fs::read(dl.join("W [x].mkv")).unwrap(), b"fusion");
        assert!(!dl.join("W [x].webm").exists());
        // Passe finale ratée (miniature impossible…) : la vidéo complète est gardée.
        std::fs::write(d.join("rate"), "").unwrap();
        let (r, _) = lance(&o, sortie("P [x].mp4", &un), "P [x].mp4");
        assert_eq!(r, Ok(Outcome::Done(dl.join("P [x].mp4"))));
        assert_eq!(std::fs::read(dl.join("P [x].mp4")).unwrap(), v);
        std::fs::remove_file(d.join("rate")).unwrap();
        // Sans ffmpeg : ni sous-titres, ni miniature, ni chapitres, pas de passe finale.
        let sans = Outils {
            ffmpeg: None,
            ..o.clone()
        };
        let (r, args) = lance(&sans, sortie("N [x].mp4", &un), "N [x].mp4");
        assert_eq!(r, Ok(Outcome::Done(dl.join("N [x].mp4"))));
        assert_eq!(args.len(), 1);
        assert!(args[0].contains("--merge-output-format mp4"));
        assert!(!args[0].contains("--embed") && !args[0].contains("subs"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn liens_de_listes() {
        use GenreListe::*;
        let pl = "https://www.youtube.com/playlist?list=PLRqwX-V7Uu6ZiZxtDDRCi6uhfTH4FilpH";
        let cas: &[(&str, Option<(GenreListe, &str)>)] = &[
            (pl, Some((Liste, pl))),
            (
                "https://www.youtube.com/watch?v=17WoOqgXsRM&list=PLx&index=2",
                Some((
                    VideoDeListe,
                    "https://www.youtube.com/watch?v=17WoOqgXsRM&list=PLx&index=2",
                )),
            ),
            (
                "https://youtu.be/17WoOqgXsRM?list=PLx",
                Some((VideoDeListe, "https://youtu.be/17WoOqgXsRM?list=PLx")),
            ),
            (
                "https://www.youtube.com/@TheCodingTrain",
                Some((Liste, "https://www.youtube.com/@TheCodingTrain/videos")),
            ),
            (
                "https://m.youtube.com/@TheCodingTrain/",
                Some((Liste, "https://www.youtube.com/@TheCodingTrain/videos")),
            ),
            (
                "https://www.youtube.com/@TheCodingTrain/videos",
                Some((Liste, "https://www.youtube.com/@TheCodingTrain/videos")),
            ),
            (
                "https://www.youtube.com/@TheCodingTrain/shorts",
                Some((Liste, "https://www.youtube.com/@TheCodingTrain/shorts")),
            ),
            (
                "https://www.youtube.com/channel/UCvjgXvBlbQiydffZU7m1_aw",
                Some((
                    Liste,
                    "https://www.youtube.com/channel/UCvjgXvBlbQiydffZU7m1_aw/videos",
                )),
            ),
            (
                "https://www.youtube.com/c/TheCodingTrain/videos?view=0",
                Some((
                    Liste,
                    "https://www.youtube.com/c/TheCodingTrain/videos?view=0",
                )),
            ),
            (
                "https://www.twitch.tv/riotgames/videos",
                Some((Liste, "https://www.twitch.tv/riotgames/videos")),
            ),
            (
                "https://www.twitch.tv/zerator/videos?filter=archives&sort=time",
                Some((
                    Liste,
                    "https://www.twitch.tv/zerator/videos?filter=archives&sort=time",
                )),
            ),
            // Une seule vidéo, ou pas une liste de vidéos.
            ("https://www.youtube.com/watch?v=17WoOqgXsRM", None),
            ("https://www.youtube.com/watch?list=PLx", None),
            ("https://www.youtube.com/playlist", None),
            ("https://www.youtube.com/shorts/abcdefghijk", None),
            ("https://www.youtube.com/@TheCodingTrain/community", None),
            ("https://www.youtube.com/@TheCodingTrain/playlists", None),
            ("https://www.youtube.com/", None),
            ("https://www.youtube.com/feed/subscriptions", None),
            ("https://www.twitch.tv/videos/2928653119", None),
            ("https://www.twitch.tv/videos/videos", None),
            ("https://www.twitch.tv/riotgames", None),
            ("https://youtube.com.evil.example/playlist?list=PLx", None),
            ("https://evil.example/@x/videos", None),
            ("ftp://www.youtube.com/playlist?list=PLx", None),
        ];
        for (u, attendu) in cas {
            assert_eq!(liste(u), attendu.map(|(g, l)| (g, l.to_string())), "{u}");
        }
    }

    /// Sortie réelle de `yt-dlp --flat-playlist -J` (YouTube, playlist publique, 3 premières
    /// entrées, 2026-11-28, miniatures retirées).
    const PLAYLIST: &[u8] = include_bytes!("../tests/fixtures/ytdlp_playlist.json");

    #[test]
    fn liste_d_une_vraie_sortie() {
        let p = playlist_depuis_json(PLAYLIST, 0).unwrap();
        assert_eq!(p.titre, "Coding Challenges");
        assert!(!p.suite);
        assert_eq!(p.lues, 3);
        assert_eq!(p.videos.len(), 3);
        assert_eq!(
            p.videos[0],
            VideoListe {
                url: "https://www.youtube.com/watch?v=17WoOqgXsRM".into(),
                titre: "Coding Challenge 1: Starfield Simulation".into(),
                duree: Some(834.0),
                numero: 1,
            }
        );
        assert_eq!(p.videos[2].numero, 3);
        // Rediffusions Twitch (url_transparent), onglets d'une chaîne (listes sans lien),
        // entrée nulle, lien non http, suite de la liste (numéros à partir de `debut`).
        let j = r#"{"title":"riotgames - All Videos","entries":[
            {"_type":"url_transparent","url":"https://www.twitch.tv/videos/2928653119","title":" GEN vs BLG ","duration":33077.0},
            {"_type":"playlist","title":"The Coding Train - Videos"},
            {"_type":"url","url":"https://www.youtube.com/@x/videos","title":"onglet"},
            null,
            {"url":"javascript:alert(1)","title":"x"},
            {"url":"https://www.twitch.tv/videos/2","title":""}]}"#;
        let p = playlist_depuis_json(j.as_bytes(), 500).unwrap();
        let v: Vec<_> = p
            .videos
            .iter()
            .map(|v| (v.numero, v.titre.as_str(), v.duree))
            .collect();
        assert_eq!(
            v,
            [
                (501, "GEN vs BLG", Some(33077.0)),
                (506, "https://www.twitch.tv/videos/2", None)
            ]
        );
        assert_eq!(p.lues, 506);
        // Plus de LISTE_MAX entrées : la dernière dit seulement qu'il y a une suite.
        let e: Vec<String> = (0..=LISTE_MAX)
            .map(|i| format!(r#"{{"url":"https://www.youtube.com/watch?v={i}"}}"#))
            .collect();
        let p = playlist_depuis_json(format!(r#"{{"entries":[{}]}}"#, e.join(",")).as_bytes(), 0)
            .unwrap();
        assert!(p.suite);
        assert_eq!((p.videos.len(), p.lues), (LISTE_MAX, LISTE_MAX));
        assert_eq!(p.titre, "playlist");
    }

    #[test]
    fn nom_de_dossier() {
        assert_eq!(nom_dossier("Coding Challenges"), "Coding Challenges");
        assert_eq!(nom_dossier("a/b\\c: d?*"), "a_b_c_ d__");
        assert_eq!(nom_dossier("../.."), "_");
        assert_eq!(nom_dossier("  ...  "), "playlist");
        assert_eq!(nom_dossier(""), "playlist");
        assert_eq!(nom_dossier("NUL"), "_NUL");
        assert_eq!(nom_dossier("Fin. "), "Fin");
        assert_eq!(nom_dossier("\u{202E}gpj.exe"), "gpj.exe");
        let long = "é".repeat(80) + " suite";
        let n = nom_dossier(&long);
        assert!(n.len() <= 100 && n.chars().all(|c| c == 'é'), "{n}");
        // Coupe qui laisse un point final : retiré.
        let n = nom_dossier(&("a".repeat(99) + ".b"));
        assert_eq!(n, "a".repeat(99));
    }

    /// Faux yt-dlp : la liste est lue avec `--flat-playlist --yes-playlist` et ses bornes ;
    /// erreurs connues traduites ; arrêt = processus tué.
    #[cfg(unix)]
    #[test]
    fn liste_par_un_faux_ytdlp() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("fluxdl-test-liste-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let yt = d.join("yt-dlp");
        std::fs::write(
            &yt,
            format!(
                "#!/bin/sh\necho \"$@\" > '{0}/args.txt'\ncase \"$*\" in\n*prive*) echo 'ERROR: [youtube:tab] PLp: YouTube said: This playlist is private' >&2; echo null; exit 1 ;;\n*absente*) echo 'ERROR: [youtube:tab] PLa: YouTube said: The playlist does not exist.' >&2; exit 1 ;;\n*vide*) echo '{{\"title\":\"V\",\"entries\":[]}}' ;;\n*lente*) sleep 30 ;;\n*) cat '{0}/liste.json' ;;\nesac\n",
                d.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&yt, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(d.join("liste.json"), PLAYLIST).unwrap();
        let o = Outils {
            ytdlp: Some(yt),
            ..Default::default()
        };
        let non = AtomicBool::new(false);
        let p = lire_liste(
            &o,
            "https://www.youtube.com/playlist?list=PLx",
            None,
            0,
            &non,
        )
        .unwrap();
        assert_eq!(p.videos.len(), 3);
        let a = std::fs::read_to_string(d.join("args.txt")).unwrap();
        assert!(a.starts_with("--flat-playlist -J --ignore-config"), "{a}");
        assert!(
            a.contains("--no-playlist")
                && a.contains("--yes-playlist --playlist-start 1 --playlist-end 501 -- https://www.youtube.com/playlist?list=PLx"),
            "{a}"
        );
        lire_liste(&o, "https://x/suite", None, 500, &non).unwrap();
        let a = std::fs::read_to_string(d.join("args.txt")).unwrap();
        assert!(
            a.contains("--playlist-start 501 --playlist-end 1001"),
            "{a}"
        );
        let err = |u: &str| lire_liste(&o, u, None, 0, &non).unwrap_err();
        assert_eq!(err("https://x/prive"), ERR_LISTE_PRIVEE);
        assert_eq!(err("https://x/absente"), ERR_LISTE_INTROUVABLE);
        assert_eq!(err("https://x/vide"), ERR_LISTE_VIDE);
        // Suite vide : pas une erreur.
        assert!(lire_liste(&o, "https://x/vide", None, 500, &non)
            .unwrap()
            .videos
            .is_empty());
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            s2.store(true, Ordering::Relaxed);
        });
        let t = std::time::Instant::now();
        assert!(lire_liste(&o, "https://x/lente", None, 0, &stop).is_err());
        assert!(t.elapsed() < Duration::from_secs(5));
        let _ = std::fs::remove_dir_all(&d);
    }
}
