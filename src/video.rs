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
use std::io::{BufRead, BufReader};
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
            Qualite::P1080 => vec![
                "-f",
                "bv*[height<=1080]+ba/b[height<=1080]/b",
                "--merge-output-format",
                "mp4",
            ],
            Qualite::P720 => vec![
                "-f",
                "bv*[height<=720]+ba/b[height<=720]/b",
                "--merge-output-format",
                "mp4",
            ],
            // Pas de son seul (formats image + son ensemble) : son extrait du meilleur format
            Qualite::M4a => vec!["-f", "ba[ext=m4a]/ba/b", "-x", "--audio-format", "m4a"],
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
    ext: Option<String>,
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
                let s = match q {
                    Qualite::M4a => j
                        .formats
                        .iter()
                        .filter(|f| f.son() && f.ext.as_deref() == Some("m4a"))
                        .max_by_key(tbr)
                        .or(son),
                    _ => son,
                };
                return (q, a_du_son, s.and_then(Format::taille));
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
    Ok(Infos {
        titre: j.title.unwrap_or_else(|| "video".into()),
        duree: j.duration,
        hauteur_max,
        choix,
        en_direct: j.is_live == Some(true),
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
}

impl Tache {
    /// Arguments propres à la tâche : format, passage, live, nom du fichier.
    fn args(&self) -> Vec<String> {
        let mut a: Vec<String> = self.qualite.args().into_iter().map(String::from).collect();
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
    // raccourci : son seul (extraction -x), passage et live restent à yt-dlp ; les passer au
    // moteur si les liens directs donnent satisfaction.
    if tache.section.is_none() && !tache.debut && !tache.qualite.audio() {
        if let Some(d) = liens_directs(o, url, dir, tache, cookies) {
            match par_le_moteur(&d, o.ffmpeg.as_deref(), connexions, ctl) {
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
        l.size = taille;
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
        (true, None) => Ok(Outcome::Done(dir.to_path_buf())),
        (false, _) => Err(derniere_erreur(&erreurs)),
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
fn liens_directs(
    o: &Outils,
    url: &str,
    dir: &Path,
    tache: &Tache,
    cookies: Option<&Path>,
) -> Option<Directs> {
    let mut c = commande(o.ytdlp.as_ref()?);
    c.args(tache.args());
    args_communs(&mut c, o, cookies);
    let s = c
        .args(["--print", "filename", "--print", "%()j", "-P"])
        .arg(dir)
        .arg("--")
        .arg(url)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    s.status
        .success()
        .then(|| directs(&String::from_utf8_lossy(&s.stdout), dir))?
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

/// Chaque partie par le moteur (plages exigées, jauges par connexion), son d'abord : la jauge
/// finit sur l'image, la plus grosse ; puis fusion par ffmpeg (`-c copy`). Arrêt : les `.part`
/// restent, la reprise repart de là (liens neufs, même fichier distant).
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
    let mut faits = vec![PathBuf::new(); n];
    for (i, p) in d.parties.iter().enumerate().rev() {
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
        match engine::run(url, dir, Some(&nom_partie(d, i)), &opts, ctl)? {
            Outcome::Done(f) => faits[i] = f,
            Outcome::Stopped => return Ok(Outcome::Stopped),
        }
    }
    if n == 1 {
        return Ok(Outcome::Done(faits.remove(0)));
    }
    // raccourci : ffmpeg n'est pas interrompu par un arrêt (copie sans réencodage, rapide).
    let sortie = fusion(d);
    let ok = commande(&ff)
        .args(["-nostdin", "-v", "error", "-y", "-i"])
        .arg(&faits[0])
        .arg("-i")
        .arg(&faits[1])
        .args(["-map", "0:v:0", "-map", "1:a:0", "-c", "copy"])
        .arg(&sortie)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
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
        assert_eq!(c(Qualite::M4a), Some(1_000_000));
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
        };
        let lance = |sortie: String| {
            std::fs::write(d.join("sortie.txt"), sortie).unwrap();
            let ctl = Arc::new(Control::default());
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
}
