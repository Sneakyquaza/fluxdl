//! Moteur de téléchargement multi-connexions, sans UI.
//!
//! Le fichier est découpé en morceaux (`Segment`) téléchargés en parallèle par N connexions.
//! Une connexion qui finit tôt prend la moitié du plus gros morceau restant (vol de travail) :
//! aucune connexion ne reste à rien faire tant qu'il reste au moins 2 × `min_chunk`.
//! Reprise : fichier `.part` préalloué + état `.part.json` (morceaux faits) écrit de façon
//! atomique toutes les 500 ms, après `sync_data` du `.part` : un `kill -9` ou une coupure de
//! courant ne fait perdre que la dernière demi-seconde.

mod probe;
pub use probe::{name_from_disposition, name_from_url, probe, sanitize};

use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub const MAX_CONNECTIONS: usize = 32;
/// Taille d'une lecture réseau ; un vol laisse toujours au moins ça au volé (voir `claim`).
const BUF: usize = 64 * 1024;
const SAVE_EVERY: Duration = Duration::from_millis(500);

#[derive(Clone, Debug)]
pub struct Options {
    /// Connexions simultanées (1 à 32).
    pub connections: usize,
    /// Plus petit morceau qu'on découpe ou qu'on vole.
    pub min_chunk: u64,
    /// Limite de débit globale en octets/s (0 = aucune).
    pub speed_limit: u64,
    /// Empreinte attendue (hex), vérifiée à la fin.
    pub expected_sha256: Option<String>,
    /// Échecs de suite sans progrès avant qu'une connexion abandonne.
    pub max_retries: u32,
    /// En-têtes ajoutés à chaque requête (Referer, Cookie venus de l'extension).
    pub headers: Vec<(String, String)>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            connections: 8,
            min_chunk: 1 << 20,
            speed_limit: 0,
            expected_sha256: None,
            max_retries: 6,
            headers: Vec::new(),
        }
    }
}

/// Ce que le serveur dit du fichier.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Probe {
    pub size: Option<u64>,
    pub ranges: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub file_name: String,
    pub content_type: Option<String>,
}

/// Morceau `[start, end)` dont `[start, pos)` est écrit sur le disque.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Segment {
    pub start: u64,
    pub pos: u64,
    pub end: u64,
    /// Connexion qui le télécharge.
    #[serde(skip)]
    pub worker: Option<usize>,
    /// Né d'un vol (« fini tôt ? prends la moitié »).
    #[serde(skip)]
    pub stolen: bool,
}

impl Segment {
    pub fn remaining(&self) -> u64 {
        self.end.saturating_sub(self.pos)
    }
}

#[derive(Clone, Debug, Default)]
pub struct WorkerStat {
    /// Octets reçus par cette connexion pendant la session.
    pub bytes: u64,
    pub segment: Option<usize>,
    pub retries: u32,
}

/// État en direct, lu par l'UI (`Control::snapshot`).
#[derive(Clone, Debug, Default)]
pub struct Live {
    pub file_name: Option<String>,
    pub size: Option<u64>,
    /// Téléchargement découpé (plages acceptées et taille connue).
    pub ranges: bool,
    pub segments: Vec<Segment>,
    pub workers: Vec<WorkerStat>,
    pub steals: u32,
    /// Octets reçus pendant cette session (hors reprise).
    pub received: u64,
    /// Le fichier distant a changé depuis la dernière fois : repris de zéro.
    pub restarted: bool,
    pub verifying: bool,
    /// Torrents : pairs connectés, octets envoyés, partage en cours après la fin.
    pub peers: u32,
    pub uploaded: u64,
    /// Débit d'envoi (octets/s).
    pub up_speed: f64,
    pub seeding: bool,
    /// Torrents : carte des pièces réduite (0 = rien, 255 = tout reçu) ; vide sinon.
    pub pieces: Vec<u8>,
}

impl Live {
    /// Octets écrits sur le disque (reprise comprise).
    pub fn done(&self) -> u64 {
        self.segments.iter().map(|s| s.pos - s.start).sum()
    }
}

/// Lien entre un téléchargement et ceux qui le suivent ou l'arrêtent.
#[derive(Default)]
pub struct Control {
    pub stop: AtomicBool,
    live: Mutex<Live>,
}

impl Control {
    pub fn with_live(live: Live) -> Self {
        Control {
            stop: AtomicBool::new(false),
            live: Mutex::new(live),
        }
    }
    pub fn snapshot(&self) -> Live {
        self.lock().clone()
    }
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
    /// État en direct à modifier (yt-dlp et torrents remplissent le même `Live`).
    pub fn lock(&self) -> MutexGuard<'_, Live> {
        self.live.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Done(PathBuf),
    /// Arrêt demandé : l'état est sauvegardé, `run` reprendra où il s'est arrêté.
    Stopped,
}

/// Fichier d'état à côté du `.part`.
#[derive(Serialize, Deserialize)]
struct Saved {
    url: String,
    size: Option<u64>,
    etag: Option<String>,
    last_modified: Option<String>,
    segments: Vec<Segment>,
}

pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(30))
        .redirects(8)
        .user_agent(&format!(
            "{}/{}",
            crate::APP_NAME,
            env!("CARGO_PKG_VERSION")
        ))
        .build()
}

pub fn part_path(target: &Path) -> PathBuf {
    with_suffix(target, ".part")
}

pub fn state_path(target: &Path) -> PathBuf {
    with_suffix(target, ".part.json")
}

fn with_suffix(p: &Path, s: &str) -> PathBuf {
    let mut o = p.as_os_str().to_owned();
    o.push(s);
    PathBuf::from(o)
}

/// Erreur de `run` quand le lien mène à une page web (l'UI propose alors yt-dlp).
pub const ERR_PAGE: &str = "This link is a web page, not a file.";

/// Télécharge `url` dans `dir`. `name` : nom déjà choisi (reprise) ; sinon celui du serveur,
/// rendu unique (« nom (1).ext »). Bloquant : à lancer dans un fil.
pub fn run(
    url: &str,
    dir: &Path,
    name: Option<&str>,
    opts: &Options,
    ctl: &Control,
) -> Result<Outcome, String> {
    let agent = agent();
    let p = probe(&agent, url, &opts.headers)?;
    if est_page(&p, url) {
        return Err(ERR_PAGE.into());
    }
    let name = match name {
        Some(n) => n.to_string(),
        None => unique_name(dir, &p.file_name),
    };
    let target = dir.join(&name);
    ctl.lock().file_name = Some(name);
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    match attempt(&agent, url, &p, &target, opts, ctl, false)? {
        // Un serveur a répondu 200 à une plage : tout reprendre sur une seule connexion.
        Step::Fallback => match attempt(&agent, url, &p, &target, opts, ctl, true)? {
            Step::Fallback => Err("server ignores ranges".into()),
            Step::Done(o) => Ok(o),
        },
        Step::Done(o) => Ok(o),
    }
}

/// Réponse HTML à un lien qui ne finit pas par .htm(l) : une page (vidéo, partage…), pas un
/// fichier.
fn est_page(p: &Probe, url: &str) -> bool {
    let chemin = url
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    est_html(p.content_type.as_deref()) && !chemin.ends_with(".html") && !chemin.ends_with(".htm")
}

/// Type de contenu HTML (`text/html`, `application/xhtml+xml`).
fn est_html(c: Option<&str>) -> bool {
    c.is_some_and(|c| {
        let c = c.trim_start().to_ascii_lowercase();
        c.starts_with("text/html") || c.starts_with("application/xhtml+xml")
    })
}

enum Step {
    Done(Outcome),
    Fallback,
}

/// Données partagées par les connexions d'une tentative.
struct Shared<'a> {
    agent: &'a ureq::Agent,
    url: &'a str,
    file: &'a File,
    probe: &'a Probe,
    opts: &'a Options,
    ctl: &'a Control,
    ranged: bool,
    started: Instant,
    /// Arrêt de toutes les connexions (erreur fatale ou repli).
    abort: AtomicBool,
    fallback: AtomicBool,
    error: Mutex<Option<String>>,
}

impl Shared<'_> {
    fn halted(&self) -> bool {
        self.ctl.stop.load(Ordering::Relaxed) || self.abort.load(Ordering::Relaxed)
    }
    fn set_error(&self, e: String) {
        *self.error.lock().unwrap_or_else(|p| p.into_inner()) = Some(e);
    }
}

fn attempt(
    agent: &ureq::Agent,
    url: &str,
    p: &Probe,
    target: &Path,
    opts: &Options,
    ctl: &Control,
    force_single: bool,
) -> Result<Step, String> {
    let part = part_path(target);
    let side = state_path(target);
    let ranged = p.ranges && p.size.is_some() && !force_single;
    let mut restarted = false;
    let mut segments = None;
    if ranged {
        if let Some(s) = load(&side) {
            let len = fs::metadata(&part).map(|m| m.len()).ok();
            if s.size == p.size
                && s.etag == p.etag
                && s.last_modified == p.last_modified
                && len == p.size
            {
                segments = Some(s.segments);
            } else {
                restarted = true;
            }
        }
    } else {
        // Pas de reprise possible sans plages : un ancien état ne sert plus.
        let _ = fs::remove_file(&side);
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(segments.is_none())
        .open(&part)
        .map_err(|e| format!("{}: {e}", part.display()))?;
    if segments.is_none() {
        if let Some(n) = p.size {
            file.set_len(n).map_err(|e| format!("preallocate: {e}"))?;
        }
    }
    let n = if ranged {
        opts.connections.clamp(1, MAX_CONNECTIONS)
    } else {
        1
    };
    let segments = segments.unwrap_or_else(|| match (ranged, p.size) {
        (true, Some(size)) => split(size, n, opts.min_chunk),
        _ => vec![Segment {
            end: p.size.unwrap_or(u64::MAX),
            ..Default::default()
        }],
    });
    {
        let mut l = ctl.lock();
        l.size = p.size;
        l.ranges = ranged;
        l.segments = segments;
        l.workers = vec![WorkerStat::default(); n];
        l.steals = 0;
        l.received = 0;
        l.restarted |= restarted;
        l.verifying = false;
    }
    let sh = Shared {
        agent,
        url,
        file: &file,
        probe: p,
        opts,
        ctl,
        ranged,
        started: Instant::now(),
        abort: AtomicBool::new(false),
        fallback: AtomicBool::new(false),
        error: Mutex::new(None),
    };
    std::thread::scope(|s| {
        let shr = &sh;
        let fils: Vec<_> = (0..n).map(|w| s.spawn(move || worker(shr, w))).collect();
        let mut dernier = Instant::now();
        while !fils.iter().all(|f| f.is_finished()) {
            std::thread::sleep(Duration::from_millis(20));
            if ranged && dernier.elapsed() >= SAVE_EVERY {
                save(&sh, &side);
                dernier = Instant::now();
            }
        }
    });
    if sh.fallback.load(Ordering::Relaxed) && !ctl.stop.load(Ordering::Relaxed) {
        return Ok(Step::Fallback);
    }
    let complete = ctl.lock().segments.iter().all(|s| s.remaining() == 0);
    if !complete {
        if ranged {
            save(&sh, &side);
        }
        let err = sh.error.lock().unwrap_or_else(|e| e.into_inner()).take();
        return match err {
            Some(e) => Err(e),
            None if ctl.stop.load(Ordering::Relaxed) => Ok(Step::Done(Outcome::Stopped)),
            None => Err("incomplete download".into()),
        };
    }
    file.sync_all().map_err(|e| format!("sync: {e}"))?;
    drop(file);
    if let Some(attendu) = &opts.expected_sha256 {
        ctl.lock().verifying = true;
        let h = sha256_file(&part).map_err(|e| format!("sha256: {e}"))?;
        ctl.lock().verifying = false;
        if !h.eq_ignore_ascii_case(attendu.trim()) {
            // Recommencer de zéro au prochain essai : l'état ne sert plus.
            let _ = fs::remove_file(&side);
            return Err(format!("sha256 mismatch: got {h}"));
        }
    }
    fs::rename(&part, target).map_err(|e| format!("rename: {e}"))?;
    let _ = fs::remove_file(&side);
    Ok(Step::Done(Outcome::Done(target.to_path_buf())))
}

/// Découpage initial en `n` morceaux d'au moins `min_chunk` (sauf fichier plus petit).
pub fn split(size: u64, n: usize, min_chunk: u64) -> Vec<Segment> {
    let parts = (size / min_chunk.max(1)).clamp(1, n.max(1) as u64);
    (0..parts)
        .map(|i| {
            let a = size * i / parts;
            Segment {
                start: a,
                pos: a,
                end: size * (i + 1) / parts,
                ..Default::default()
            }
        })
        .collect()
}

/// Morceau suivant pour la connexion `w` : un morceau libre, sinon la moitié haute du plus gros
/// morceau en cours. Invariant : le volé garde au moins `BUF` octets, donc l'écriture qu'il a
/// peut-être en vol (≤ `BUF`, bornée par l'ancienne fin) ne déborde jamais sur la part volée.
fn claim(sh: &Shared, w: usize) -> Option<usize> {
    let mut l = sh.ctl.lock();
    let i = match l
        .segments
        .iter()
        .position(|s| s.worker.is_none() && s.remaining() > 0)
    {
        Some(i) => i,
        None if sh.ranged => {
            let (i, s) = l
                .segments
                .iter()
                .enumerate()
                .filter(|(_, s)| s.worker.is_some())
                .max_by_key(|(_, s)| s.remaining())?;
            let reste = s.remaining();
            if reste < 2 * sh.opts.min_chunk.max(BUF as u64) {
                return None;
            }
            let mid = s.pos + reste / 2;
            let end = s.end;
            l.segments[i].end = mid;
            l.segments.push(Segment {
                start: mid,
                pos: mid,
                end,
                worker: None,
                stolen: true,
            });
            l.steals += 1;
            l.segments.len() - 1
        }
        None => return None,
    };
    l.segments[i].worker = Some(w);
    l.workers[w].segment = Some(i);
    Some(i)
}

enum FetchErr {
    Retry(String),
    Fatal(String),
    Fallback,
}

fn worker(sh: &Shared, w: usize) {
    let mut echecs = 0u32;
    let mut courant = None;
    while !sh.halted() {
        let i = match courant {
            Some(i) => i,
            None => match claim(sh, w) {
                Some(i) => i,
                None => return,
            },
        };
        courant = Some(i);
        let avant = sh.ctl.lock().segments[i].pos;
        match fetch(sh, w, i) {
            Ok(()) => {
                courant = None;
                echecs = 0;
            }
            Err(FetchErr::Fallback) => {
                sh.fallback.store(true, Ordering::Relaxed);
                sh.abort.store(true, Ordering::Relaxed);
            }
            Err(FetchErr::Fatal(e)) => {
                sh.set_error(e);
                sh.abort.store(true, Ordering::Relaxed);
            }
            Err(FetchErr::Retry(e)) => {
                let progres = sh.ctl.lock().segments[i].pos > avant;
                echecs = if progres { 1 } else { echecs + 1 };
                sh.ctl.lock().workers[w].retries += 1;
                if echecs > sh.opts.max_retries {
                    // Rendre le morceau : une autre connexion pourra le prendre.
                    let mut l = sh.ctl.lock();
                    l.segments[i].worker = None;
                    l.workers[w].segment = None;
                    drop(l);
                    sh.set_error(e);
                    return;
                }
                // Attente exponentielle : 250 ms, 500 ms… 8 s au plus, interrompue par un arrêt.
                let fin = Instant::now() + Duration::from_millis(250 << (echecs - 1).min(5));
                while Instant::now() < fin && !sh.halted() {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }
}

fn classify(e: ureq::Error) -> FetchErr {
    match e {
        ureq::Error::Status(c, _) if c == 408 || c == 429 || c >= 500 => {
            FetchErr::Retry(format!("HTTP {c}"))
        }
        ureq::Error::Status(c, _) => FetchErr::Fatal(format!("HTTP {c}")),
        e => FetchErr::Retry(e.to_string()),
    }
}

/// Télécharge le morceau `i` depuis sa position jusqu'à sa fin (qui peut reculer en cours de
/// route si on lui vole sa moitié haute).
fn fetch(sh: &Shared, w: usize, i: usize) -> Result<(), FetchErr> {
    let (mut pos, end) = {
        let mut l = sh.ctl.lock();
        let s = &mut l.segments[i];
        if !sh.ranged {
            // Sans plages, chaque essai repart du début.
            s.pos = s.start;
        }
        (s.pos, s.end)
    };
    if end != u64::MAX && pos >= end {
        return Ok(());
    }
    let mut req = sh.agent.get(sh.url);
    for (k, v) in &sh.opts.headers {
        req = req.set(k, v);
    }
    if sh.ranged {
        req = req.set("Range", &format!("bytes={pos}-{}", end - 1));
    }
    let resp = req.call().map_err(classify)?;
    // Une page web à la place du fichier (session expirée, anti-robot…) : ne rien écrire.
    if est_html(resp.header("Content-Type")) && !est_html(sh.probe.content_type.as_deref()) {
        return Err(FetchErr::Fatal(ERR_PAGE.into()));
    }
    let change = || FetchErr::Fatal("remote file changed".into());
    if sh.ranged {
        if resp.status() != 206 {
            return Err(FetchErr::Fallback);
        }
        let cr = resp.header("Content-Range").unwrap_or("");
        let debut = cr
            .strip_prefix("bytes ")
            .and_then(|r| r.split('-').next())
            .and_then(|v| v.trim().parse::<u64>().ok());
        let total = cr.rsplit('/').next().and_then(|t| t.trim().parse().ok());
        if total != sh.probe.size {
            return Err(change());
        }
        if debut != Some(pos) {
            return Err(FetchErr::Retry(format!("bad Content-Range: {cr}")));
        }
    }
    if let (Some(a), Some(b)) = (&sh.probe.etag, resp.header("ETag")) {
        if a != b {
            return Err(change());
        }
    }
    let mut r = resp.into_reader();
    let mut buf = vec![0u8; BUF];
    loop {
        if sh.halted() {
            return Ok(());
        }
        let n = match r.read(&mut buf) {
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(FetchErr::Retry(e.to_string())),
        };
        let end = sh.ctl.lock().segments[i].end;
        if n == 0 {
            if end == u64::MAX {
                // Taille inconnue : la fin du flux fixe la taille.
                let mut l = sh.ctl.lock();
                l.segments[i].end = pos;
                l.size = Some(pos);
                return Ok(());
            }
            return Err(FetchErr::Retry("connection closed early".into()));
        }
        let k = (n as u64).min(end - pos) as usize;
        write_at(sh.file, &buf[..k], pos).map_err(|e| FetchErr::Fatal(format!("write: {e}")))?;
        pos += k as u64;
        let attente = {
            let mut l = sh.ctl.lock();
            l.segments[i].pos = pos;
            l.received += k as u64;
            l.workers[w].bytes += k as u64;
            if pos >= l.segments[i].end {
                return Ok(());
            }
            limit_wait(sh, l.received)
        };
        if let Some(d) = attente {
            std::thread::sleep(d);
        }
    }
}

/// Limite globale : attente pour que `recu` octets ne dépassent pas le débit permis.
fn limit_wait(sh: &Shared, recu: u64) -> Option<Duration> {
    let debit = sh.opts.speed_limit;
    if debit == 0 {
        return None;
    }
    let prevu = Duration::from_secs_f64(recu as f64 / debit as f64);
    prevu.checked_sub(sh.started.elapsed())
}

#[cfg(unix)]
fn write_at(f: &File, buf: &[u8], off: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    f.write_all_at(buf, off)
}

#[cfg(windows)]
fn write_at(f: &File, mut buf: &[u8], mut off: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        let n = f.seek_write(buf, off)?;
        if n == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        buf = &buf[n..];
        off += n as u64;
    }
    Ok(())
}

/// Instantané des morceaux, `sync_data` du `.part` (les octets annoncés sont sur le disque),
/// puis état écrit dans un fichier temporaire renommé (jamais d'état à moitié écrit).
fn save(sh: &Shared, side: &Path) {
    let segments = sh.ctl.lock().segments.clone();
    if sh.file.sync_data().is_err() {
        return;
    }
    let s = Saved {
        url: sh.url.to_string(),
        size: sh.probe.size,
        etag: sh.probe.etag.clone(),
        last_modified: sh.probe.last_modified.clone(),
        segments,
    };
    let Ok(json) = serde_json::to_vec(&s) else {
        return;
    };
    let _ = write_atomic(side, &json);
}

pub fn write_atomic(p: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = with_suffix(p, ".tmp");
    let mut f = File::create(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    drop(f);
    fs::rename(&tmp, p)
}

fn load(side: &Path) -> Option<Saved> {
    serde_json::from_slice(&fs::read(side).ok()?).ok()
}

/// « nom.ext », sinon « nom (1).ext »… libre (ni fichier fini ni `.part`).
fn unique_name(dir: &Path, name: &str) -> String {
    let libre = |n: &str| !dir.join(n).exists() && !part_path(&dir.join(n)).exists();
    if libre(name) {
        return name.to_string();
    }
    let (base, ext) = match name.rsplit_once('.') {
        Some((b, e)) if !b.is_empty() => (b, format!(".{e}")),
        _ => (name, String::new()),
    };
    (1..)
        .map(|k| format!("{base} ({k}){ext}"))
        .find(|n| libre(n))
        .unwrap_or_default()
}

pub fn sha256_file(p: &Path) -> io::Result<String> {
    let mut f = File::open(p)?;
    let mut c = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        c.update(&buf[..n]);
    }
    Ok(hex(c.finish().as_ref()))
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Comparatif : télécharge `url` avec `connections` connexions dans un dossier temporaire
/// pendant `max` au plus, puis efface tout. Renvoie le débit moyen en octets/s.
pub fn measure(url: &str, connections: usize, max: Duration, ctl: &Control) -> Result<f64, String> {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "{}-bench-{}-{}",
        crate::APP_NAME,
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let opts = Options {
        connections,
        ..Default::default()
    };
    let debut = Instant::now();
    let r = std::thread::scope(|s| {
        let f = s.spawn(|| run(url, &dir, None, &opts, ctl));
        while !f.is_finished() {
            if debut.elapsed() >= max {
                ctl.request_stop();
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        f.join().unwrap_or_else(|_| Err("panic".into()))
    });
    let duree = debut.elapsed().as_secs_f64();
    let _ = fs::remove_dir_all(&dir);
    r?;
    Ok(ctl.snapshot().received as f64 / duree.max(1e-3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoupage() {
        let s = split(10 << 20, 8, 1 << 20);
        assert_eq!(s.len(), 8);
        assert_eq!(s[0].start, 0);
        assert_eq!(s[7].end, 10 << 20);
        assert!(s.windows(2).all(|w| w[0].end == w[1].start));
        // Petit fichier : pas de morceau sous `min_chunk`.
        assert_eq!(split(3 << 20, 32, 1 << 20).len(), 3);
        assert_eq!(split(0, 8, 1 << 20).len(), 1);
    }

    #[test]
    fn page_web_ou_fichier() {
        let p = |c: &str| Probe {
            content_type: Some(c.into()),
            ..Default::default()
        };
        assert!(est_page(
            &p("text/html; charset=utf-8"),
            "https://www.youtube.com/watch?v=x"
        ));
        assert!(!est_page(&p("text/html"), "https://a.b/doc/page.html?x=1"));
        assert!(!est_page(&p("application/zip"), "https://a.b/f"));
        assert!(!est_page(&Probe::default(), "https://a.b/f"));
        assert!(est_html(Some("application/xhtml+xml")));
        assert!(est_html(Some(" Text/HTML; charset=UTF-8")));
        assert!(!est_html(Some("application/octet-stream")));
        assert!(!est_html(None));
    }

    #[test]
    fn noms_uniques() {
        let d = std::env::temp_dir().join(format!("{}-un-{}", crate::APP_NAME, std::process::id()));
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("a.bin"), b"x").unwrap();
        fs::write(d.join("a (1).bin.part"), b"x").unwrap();
        assert_eq!(unique_name(&d, "a.bin"), "a (2).bin");
        assert_eq!(unique_name(&d, "b"), "b");
        let _ = fs::remove_dir_all(&d);
    }
}
