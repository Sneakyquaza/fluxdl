//! Tests du moteur contre un petit serveur HTTP local (TcpListener) : plages, débit bridé par
//! connexion, serveur qui ignore les plages ou ment, coupures, changement d'ETag, reprise.

use dlcore::engine::{self, Control, Options, Outcome};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Cfg {
    data: Mutex<Arc<Vec<u8>>>,
    etag: Mutex<String>,
    /// Annonce et honore les plages.
    ranges: bool,
    /// Annonce les plages, répond 206 à la sonde puis 200 à tout le reste.
    lie: bool,
    /// Pas de Content-Length (fin du corps = fermeture).
    no_length: bool,
    /// Débit max par connexion en octets/s (0 = libre).
    rate: u64,
    /// Débit des requêtes dont la plage commence à 0 (0 = `rate`).
    slow_first: u64,
    /// Réponses à couper encore, après `drop_after` octets.
    drops: AtomicU32,
    drop_after: u64,
    served: AtomicU64,
    requests: AtomicU32,
    /// Fichier virtuel de cette taille (octet = `virtuel_octet(position)`), sans `data`.
    virtuel: u64,
    /// Requêtes impaires (après la sonde) : 200 + fichier entier malgré la plage.
    full_odd: bool,
    /// Réponses à venir dont le Content-Range commence un octet trop loin.
    bad_cr: AtomicU32,
    /// Connexions simultanées permises (0 = libre) ; au-delà : 403 en HTML.
    max_conns: u32,
    actives: AtomicU32,
    /// Après la sonde : en-têtes envoyés, puis plus rien (serveur muet).
    stall: bool,
    /// La sonde elle-même ne reçoit rien (serveur muet dès la connexion).
    stall_probe: bool,
    /// `Content-Encoding: gzip` sur toutes les réponses, ou seulement sur les 206.
    gzip: bool,
    gzip_206: bool,
    /// À partir de cette requête (0 = jamais) : 403 en texte (lien signé expiré).
    expire_after: u32,
    /// ETag faible `W/` après la sonde ; ETag différent à chaque requête (CDN).
    etag_weak: bool,
    etag_varie: bool,
    /// Pas de Last-Modified.
    no_lm: bool,
    /// En-têtes reçus, une ligne « chemin | en-tête » chacun.
    vus: Mutex<Vec<String>>,
    /// Base d'un autre serveur (redirection vers un autre hôte).
    ailleurs: Mutex<String>,
    /// Plus grande plage servie (0 = libre) ; au-delà : 403 (comme googlevideo).
    max_plage: u64,
}

fn virtuel_octet(p: u64) -> u8 {
    (p ^ (p >> 9) ^ (p >> 21) ^ (p >> 33)) as u8
}

/// Décompte des connexions ouvertes du serveur.
struct Ouverte<'a>(&'a AtomicU32);
impl Drop for Ouverte<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn serve(cfg: Arc<Cfg>) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let c = cfg.clone();
            std::thread::spawn(move || {
                let _ = handle(&c, s);
            });
        }
    });
    base
}

fn handle(c: &Cfg, mut s: TcpStream) -> std::io::Result<()> {
    let mut r = BufReader::new(s.try_clone()?);
    let mut ligne = String::new();
    r.read_line(&mut ligne)?;
    let chemin = ligne.split_whitespace().nth(1).unwrap_or("/").to_string();
    let mut range = None;
    loop {
        let mut h = String::new();
        r.read_line(&mut h)?;
        let h = h.trim();
        if h.is_empty() {
            break;
        }
        c.vus.lock().unwrap().push(format!("{chemin} | {h}"));
        if let Some((k, v)) = h.split_once(':') {
            if k.eq_ignore_ascii_case("range") {
                range = v.trim().strip_prefix("bytes=").map(str::to_string);
            }
        }
    }
    if chemin == "/redirect" {
        return s.write_all(
            b"HTTP/1.1 302 Found\r\nLocation: /file.bin\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    }
    if chemin == "/redirect_ext" {
        let l = format!("{}/file.bin", c.ailleurs.lock().unwrap());
        return s.write_all(
            format!("HTTP/1.1 302 Found\r\nLocation: {l}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes(),
        );
    }
    if chemin.starts_with("/absent") {
        return s.write_all(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    }
    let n_req = c.requests.fetch_add(1, Ordering::SeqCst);
    let ouvertes = c.actives.fetch_add(1, Ordering::SeqCst) + 1;
    let _o = Ouverte(&c.actives);
    if c.max_conns > 0 && n_req > 0 && ouvertes > c.max_conns {
        return s.write_all(
            b"HTTP/1.1 403 Forbidden\r\nContent-Type: text/html\r\nContent-Length: 5\r\nConnection: close\r\n\r\nnope!",
        );
    }
    if c.stall_probe && n_req == 0 {
        std::thread::sleep(Duration::from_secs(20));
        return Ok(());
    }
    if c.expire_after > 0 && n_req >= c.expire_after {
        return s.write_all(
            b"HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nContent-Length: 6\r\nConnection: close\r\n\r\nexpire",
        );
    }
    let data = c.data.lock().unwrap().clone();
    let mut etag = format!("\"{}\"", c.etag.lock().unwrap());
    if c.etag_weak && n_req > 0 {
        etag = format!("W/{etag}");
    }
    if c.etag_varie {
        etag = format!("\"n{n_req}\"");
    }
    let total = if c.virtuel > 0 {
        c.virtuel
    } else {
        data.len() as u64
    };
    if total == 0 && c.ranges && range.is_some() {
        return s.write_all(
            b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    }
    let plage = range.as_deref().filter(|_| c.ranges).and_then(|r| {
        let (a, b) = r.split_once('-')?;
        let a: u64 = a.parse().ok()?;
        let b: u64 = if b.is_empty() {
            total - 1
        } else {
            b.parse().ok()?
        };
        Some((a, b.min(total - 1)))
    });
    let plage = plage.filter(|_| !(c.lie && n_req > 0) && !(c.full_odd && n_req % 2 == 1));
    if c.max_plage > 0 && plage.is_some_and(|(a, b)| b + 1 - a > c.max_plage) {
        return s.write_all(
            b"HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nContent-Length: 3\r\nConnection: close\r\n\r\nbig",
        );
    }
    let mut tete = String::new();
    let (a, b) = match plage {
        Some((a, b)) => {
            let decale = c
                .bad_cr
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |d| d.checked_sub(1))
                .is_ok() as u64;
            tete += &format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {}-{b}/{total}\r\n",
                a + decale
            );
            (a, b + 1)
        }
        None => {
            tete += "HTTP/1.1 200 OK\r\n";
            (0, total)
        }
    };
    if !c.no_length {
        tete += &format!("Content-Length: {}\r\n", b - a);
    }
    if c.ranges {
        tete += "Accept-Ranges: bytes\r\n";
    }
    if c.gzip || (c.gzip_206 && plage.is_some()) {
        tete += "Content-Encoding: gzip\r\n";
    }
    if !c.no_lm {
        tete += "Last-Modified: Wed, 25 Nov 2026 10:00:00 GMT\r\n";
    }
    tete += &format!("ETag: {etag}\r\nConnection: close\r\n\r\n");
    s.write_all(tete.as_bytes())?;
    if c.stall && n_req > 0 {
        std::thread::sleep(Duration::from_secs(20));
        return Ok(());
    }
    let debit = if a == 0 && c.slow_first > 0 {
        c.slow_first
    } else {
        c.rate
    };
    let coupe = c
        .drops
        .try_update(Ordering::SeqCst, Ordering::SeqCst, |d| d.checked_sub(1))
        .is_ok();
    let debut = Instant::now();
    let mut envoye = 0u64;
    let mut p = a;
    while p < b {
        if coupe && envoye >= c.drop_after {
            return s.shutdown(std::net::Shutdown::Both);
        }
        let q = (p + 16 * 1024).min(b);
        if c.virtuel > 0 {
            s.write_all(&(p..q).map(virtuel_octet).collect::<Vec<_>>())?;
        } else {
            s.write_all(&data[p as usize..q as usize])?;
        }
        c.served.fetch_add(q - p, Ordering::SeqCst);
        envoye += q - p;
        p = q;
        if debit > 0 {
            let prevu = Duration::from_secs_f64(envoye as f64 / debit as f64);
            if let Some(d) = prevu.checked_sub(debut.elapsed()) {
                std::thread::sleep(d);
            }
        }
    }
    Ok(())
}

fn donnees(n: usize, graine: u64) -> Vec<u8> {
    let mut x = graine | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

fn serveur(n: usize, f: impl FnOnce(&mut Cfg)) -> (Arc<Cfg>, String, Vec<u8>) {
    let data = donnees(n, n as u64);
    let mut c = Cfg {
        ranges: true,
        ..Default::default()
    };
    *c.data.get_mut().unwrap() = Arc::new(data.clone());
    *c.etag.get_mut().unwrap() = "v1".into();
    f(&mut c);
    let c = Arc::new(c);
    let base = serve(c.clone());
    (c, format!("{base}/file.bin"), data)
}

fn dossier(nom: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "{}-tests-{}-{nom}",
        dlcore::APP_NAME,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn sha(b: &[u8]) -> String {
    engine::hex(ring::digest::digest(&ring::digest::SHA256, b).as_ref())
}

fn opts(n: usize) -> Options {
    Options {
        connections: n,
        min_chunk: 64 * 1024,
        ..Default::default()
    }
}

fn telecharge(url: &str, d: &Path, o: &Options) -> (PathBuf, Arc<Control>) {
    let ctl = Arc::new(Control::default());
    match engine::run(url, d, None, o, &ctl).unwrap() {
        Outcome::Done(p) => (p, ctl),
        Outcome::Stopped => panic!("arrêté"),
    }
}

fn verifie(p: &Path, data: &[u8]) {
    assert_eq!(engine::sha256_file(p).unwrap(), sha(data));
    assert!(!engine::part_path(p).exists());
    assert!(!engine::state_path(p).exists());
}

#[test]
fn plages_et_redirection() {
    let (_c, url, data) = serveur(3 << 20, |_| {});
    let d = dossier("plages");
    let url = url.replace("/file.bin", "/redirect");
    let mut o = opts(8);
    o.expected_sha256 = Some(sha(&data).to_uppercase());
    let (p, ctl) = telecharge(&url, &d, &o);
    assert_eq!(p.file_name().unwrap(), "file.bin");
    verifie(&p, &data);
    let l = ctl.snapshot();
    assert!(l.ranges);
    assert!(l.segments.len() >= 8);
    // Empreinte fausse : erreur, aucun fichier final.
    o.expected_sha256 = Some("00".into());
    let e = engine::run(&url, &d, None, &o, &Arc::new(Control::default())).unwrap_err();
    assert!(e.contains("sha256"), "{e}");
}

#[test]
fn serveur_sans_plages() {
    let (_c, url, data) = serveur(1 << 20, |c| c.ranges = false);
    let d = dossier("sans_plages");
    let (p, ctl) = telecharge(&url, &d, &opts(8));
    verifie(&p, &data);
    assert!(!ctl.snapshot().ranges);
    // Taille inconnue (pas de Content-Length).
    let (_c, url, data) = serveur(700_000, |c| {
        c.ranges = false;
        c.no_length = true;
    });
    let (p, ctl) = telecharge(&url, &d, &opts(8));
    verifie(&p, &data);
    assert_eq!(ctl.snapshot().size, Some(700_000));
    // Fin du flux = taille : rien ne prouve qu'il n'a pas été coupé.
    assert!(ctl.snapshot().size_unverified);
}

#[test]
fn serveur_qui_ment_sur_les_plages() {
    let (_c, url, data) = serveur(2 << 20, |c| c.lie = true);
    let d = dossier("menteur");
    let (p, ctl) = telecharge(&url, &d, &opts(8));
    verifie(&p, &data);
    assert!(!ctl.snapshot().ranges, "repli sur une connexion");
}

#[test]
fn coupures_en_route() {
    let (c, url, data) = serveur(2 << 20, |c| {
        c.drops = AtomicU32::new(6);
        c.drop_after = 200_000;
    });
    let d = dossier("coupures");
    let (p, ctl) = telecharge(&url, &d, &opts(4));
    verifie(&p, &data);
    assert_eq!(c.drops.load(Ordering::SeqCst), 0);
    assert!(
        ctl.snapshot()
            .workers
            .iter()
            .map(|w| w.retries)
            .sum::<u32>()
            >= 1
    );
}

#[test]
fn vol_de_travail() {
    // La connexion qui part de 0 est lente : les autres finissent tôt et lui volent sa moitié.
    let taille = 4 << 20;
    let (_c, url, data) = serveur(taille, |c| c.slow_first = 256 * 1024);
    let d = dossier("vol");
    let (p, ctl) = telecharge(&url, &d, &opts(4));
    verifie(&p, &data);
    let l = ctl.snapshot();
    assert!(l.steals >= 2, "vols : {}", l.steals);
    assert!(l.segments.iter().any(|s| s.stolen));
    let max = l.workers.iter().map(|w| w.bytes).max().unwrap();
    assert!(
        max > taille as u64 / 4,
        "une connexion rapide a pris plus que sa part"
    );
}

/// Octets annoncés faits par le fichier d'état sur le disque.
fn fait_selon_etat(p: &Path) -> u64 {
    let Ok(b) = std::fs::read(p) else { return 0 };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) else {
        return 0;
    };
    v["segments"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|s| s["pos"].as_u64().unwrap() - s["start"].as_u64().unwrap())
                .sum()
        })
        .unwrap_or(0)
}

#[test]
fn reprise_apres_crash() {
    let taille = 4u64 << 20;
    let (c, url, data) = serveur(taille as usize, |c| c.rate = 512 * 1024);
    let d1 = dossier("crash1");
    let d2 = dossier("crash2");
    let cible = d1.join("file.bin");
    let ctl = Arc::new(Control::default());
    std::thread::scope(|s| {
        let f = s.spawn(|| engine::run(&url, &d1, None, &opts(4), &ctl));
        // « kill -9 » : copier l'état puis le .part tels qu'ils sont sur le disque en cours de route.
        let debut = Instant::now();
        while fait_selon_etat(&engine::state_path(&cible)) < taille * 3 / 8 {
            assert!(debut.elapsed() < Duration::from_secs(8));
            std::thread::sleep(Duration::from_millis(10));
        }
        std::fs::copy(
            engine::state_path(&cible),
            engine::state_path(&d2.join("file.bin")),
        )
        .unwrap();
        std::fs::copy(
            engine::part_path(&cible),
            engine::part_path(&d2.join("file.bin")),
        )
        .unwrap();
        ctl.request_stop();
        assert_eq!(f.join().unwrap(), Ok(Outcome::Stopped));
    });
    let avant = c.served.load(Ordering::SeqCst);
    let ctl2 = Arc::new(Control::default());
    let r = engine::run(&url, &d2, Some("file.bin"), &opts(4), &ctl2).unwrap();
    let Outcome::Done(p) = r else { panic!() };
    verifie(&p, &data);
    let reprise = c.served.load(Ordering::SeqCst) - avant;
    assert!(
        reprise < taille * 3 / 4,
        "re-téléchargé {reprise} sur {taille}"
    );
    assert!(!ctl2.snapshot().restarted);
}

#[test]
fn etag_change_entre_deux() {
    let taille = 2usize << 20;
    let (c, url, _) = serveur(taille, |c| c.rate = 512 * 1024);
    let d = dossier("etag");
    let ctl = Arc::new(Control::default());
    std::thread::scope(|s| {
        let f = s.spawn(|| engine::run(&url, &d, None, &opts(2), &ctl));
        while ctl.snapshot().done() < 512 * 1024 {
            std::thread::sleep(Duration::from_millis(10));
        }
        ctl.request_stop();
        assert_eq!(f.join().unwrap(), Ok(Outcome::Stopped));
    });
    assert!(engine::state_path(&d.join("file.bin")).exists());
    let neuves = donnees(taille, 99);
    *c.data.lock().unwrap() = Arc::new(neuves.clone());
    *c.etag.lock().unwrap() = "v2".into();
    let ctl = Arc::new(Control::default());
    let r = engine::run(&url, &d, Some("file.bin"), &opts(2), &ctl).unwrap();
    let Outcome::Done(p) = r else { panic!() };
    verifie(&p, &neuves);
    assert!(ctl.snapshot().restarted);
}

#[test]
fn acceleration_serveur_bride_par_connexion() {
    let (_c, url, data) = serveur(2 << 20, |c| c.rate = 1 << 20);
    let mesure = |n: usize, nom: &str| {
        let d = dossier(nom);
        let t = Instant::now();
        let (p, _) = telecharge(&url, &d, &opts(n));
        let t = t.elapsed().as_secs_f64();
        verifie(&p, &data);
        t
    };
    let t1 = mesure(1, "acc1");
    let t16 = mesure(16, "acc16");
    eprintln!("N=1 : {t1:.2} s, N=16 : {t16:.2} s, ×{:.1}", t1 / t16);
    assert!(t1 / t16 >= 5.0, "×{:.1}", t1 / t16);
}

#[test]
fn plage_ignoree_par_certaines_connexions() {
    // Une requête sur deux reçoit 200 + fichier entier : jamais écrit à la position de la plage.
    let (_c, url, data) = serveur(3 << 20, |c| c.full_odd = true);
    let d = dossier("plage_ignoree");
    let (p, ctl) = telecharge(&url, &d, &opts(6));
    verifie(&p, &data);
    let l = ctl.snapshot();
    assert!(l.ranges, "pas de repli sur une connexion");
    assert!(l.workers.iter().any(|w| w.error.is_some()));
}

#[test]
fn content_range_faux() {
    let (_c, url, data) = serveur(2 << 20, |c| c.bad_cr = AtomicU32::new(5));
    let d = dossier("cr_faux");
    let (p, ctl) = telecharge(&url, &d, &opts(4));
    verifie(&p, &data);
    let l = ctl.snapshot();
    assert!(l.workers.iter().any(|w| w
        .error
        .as_deref()
        .is_some_and(|e| e.contains("Content-Range"))));
}

#[test]
fn hote_limite_les_connexions() {
    // Au-delà de 3 connexions : 403 en HTML. Les connexions en trop se retirent, le reste finit.
    let (_c, url, data) = serveur(3 << 20, |c| {
        c.max_conns = 3;
        c.rate = 4 << 20;
    });
    let d = dossier("limite");
    let (p, ctl) = telecharge(&url, &d, &opts(8));
    verifie(&p, &data);
    let l = ctl.snapshot();
    let retirees = l.workers.iter().filter(|w| w.dropped).count();
    assert!((1..8).contains(&retirees), "retirées : {retirees}");
    assert!(l
        .workers
        .iter()
        .filter(|w| w.dropped)
        .all(|w| w.error.as_deref() == Some("HTTP 403")));
}

#[test]
fn serveur_muet_pause_et_annulation_rapides() {
    let (c, url, _) = serveur(4 << 20, |c| c.stall = true);
    let d = dossier("muet");
    let cible = d.join("file.bin");
    let ctl = Arc::new(Control::default());
    let (tx, rx) = std::sync::mpsc::channel();
    let (u, dd, cc) = (url.clone(), d.clone(), ctl.clone());
    std::thread::spawn(move || {
        let _ = tx.send(engine::run(&u, &dd, None, &opts(4), &cc));
    });
    let debut = Instant::now();
    while c.requests.load(Ordering::SeqCst) < 5 {
        assert!(debut.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(200));
    let t = Instant::now();
    ctl.request_stop();
    let r = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("arrêt en < 1 s");
    assert_eq!(r, Ok(Outcome::Stopped));
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    // Annulation : partiels effacés, rien n'est recréé par une connexion encore bloquée.
    std::fs::remove_file(engine::part_path(&cible)).unwrap();
    let _ = std::fs::remove_file(engine::state_path(&cible));
    std::thread::sleep(Duration::from_millis(300));
    assert!(!engine::part_path(&cible).exists());
    assert!(!engine::state_path(&cible).exists());
}

#[test]
fn gros_fichier_creux_borne() {
    // 16 Gio virtuels (> 4 Gio : positions sur 64 bits), débit bridé : quelques Mo par morceau,
    // puis pause. Le .part a la taille exacte, ne déborde jamais, et n'occupe presque rien.
    let total = 16u64 << 30;
    let (_c, url, _) = serveur(0, |c| {
        c.virtuel = total;
        c.rate = 4 << 20;
    });
    let d = dossier("gros");
    let cible = d.join("file.bin");
    let ctl = Arc::new(Control::default());
    let (tx, rx) = std::sync::mpsc::channel();
    let (u, dd, cc) = (url.clone(), d.clone(), ctl.clone());
    std::thread::spawn(move || {
        let _ = tx.send(engine::run(&u, &dd, None, &opts(8), &cc));
    });
    let debut = Instant::now();
    while ctl.snapshot().done() < 8 << 20 {
        assert!(debut.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
    ctl.request_stop();
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        Ok(Outcome::Stopped)
    );
    let part = engine::part_path(&cible);
    let m = std::fs::metadata(&part).unwrap();
    assert_eq!(m.len(), total);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert!(
            m.blocks() * 512 < 256 << 20,
            "{} o sur le disque",
            m.blocks() * 512
        );
    }
    // Chaque morceau commencé : octets exacts au début et juste avant sa position.
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(&part).unwrap();
    let l = ctl.snapshot();
    assert!(l
        .segments
        .iter()
        .all(|s| s.start <= s.pos && s.pos <= s.end && s.end <= total));
    for s in l.segments.iter().filter(|s| s.pos - s.start >= 4096) {
        for o in [s.start, s.pos - 4096] {
            let mut b = vec![0u8; 4096];
            f.seek(SeekFrom::Start(o)).unwrap();
            f.read_exact(&mut b).unwrap();
            assert!(b
                .iter()
                .enumerate()
                .all(|(k, &x)| x == virtuel_octet(o + k as u64)));
        }
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn place_insuffisante() {
    // 1 Pio annoncé : refusé avant d'écrire quoi que ce soit.
    let (_c, url, _) = serveur(0, |c| c.virtuel = 1 << 50);
    let d = dossier("place");
    let e = engine::run(&url, &d, None, &opts(4), &Arc::new(Control::default())).unwrap_err();
    assert_eq!(e, engine::ERR_ESPACE);
    let part = engine::part_path(&d.join("file.bin"));
    assert_eq!(std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0), 0);
}

/// Arrêt demandé pendant que `run` tourne ; renvoie le résultat et le délai de retour.
fn arret_pendant(url: &str, d: &Path, apres: Duration) -> (Result<Outcome, String>, Duration) {
    let ctl = Arc::new(Control::default());
    let (tx, rx) = std::sync::mpsc::channel();
    let (u, dd, cc) = (url.to_string(), d.to_path_buf(), ctl.clone());
    std::thread::spawn(move || {
        let _ = tx.send(engine::run(&u, &dd, None, &opts(4), &cc));
    });
    std::thread::sleep(apres);
    let t = Instant::now();
    ctl.request_stop();
    let r = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("arrêt en < 2 s");
    (r, t.elapsed())
}

#[test]
fn pause_pendant_la_sonde() {
    // Audit H5 : la sonde attendait 30 s un serveur muet, la pause finissait en « Échec ».
    let (_c, url, _) = serveur(1 << 20, |c| c.stall_probe = true);
    let d = dossier("sonde_muette");
    let (r, t) = arret_pendant(&url, &d, Duration::from_millis(300));
    assert_eq!(r, Ok(Outcome::Stopped));
    assert!(t < Duration::from_secs(1), "{t:?}");
}

#[test]
fn erreurs_sans_url() {
    // Audit L2 : le jeton d'un lien signé n'apparaît jamais dans le message.
    let (_c, url, _) = serveur(1000, |_| {});
    let url = url.replace("/file.bin", "/absent.bin?token=SECRET123");
    let d = dossier("sans_url");
    let e = engine::run(&url, &d, None, &opts(2), &Arc::new(Control::default())).unwrap_err();
    assert!(!e.contains("SECRET") && !e.contains("127.0.0.1"), "{e}");
    assert!(e.contains("404"), "{e}");
}

#[test]
fn reponses_compressees() {
    // Audit M3 : jamais d'octets gzip enregistrés comme fichier.
    let (c, url, _) = serveur(1 << 20, |c| c.gzip = true);
    let d = dossier("gzip");
    let e = engine::run(&url, &d, None, &opts(4), &Arc::new(Control::default())).unwrap_err();
    assert_eq!(e, engine::ERR_ENCODAGE);
    assert!(!d.join("file.bin").exists());
    assert!(c
        .vus
        .lock()
        .unwrap()
        .iter()
        .all(|l| !l.to_ascii_lowercase().contains("accept-encoding")
            || l.ends_with("Accept-Encoding: identity")));
    // Plages compressées seulement : une connexion, fichier exact.
    let (_c, url, data) = serveur(1 << 20, |c| c.gzip_206 = true);
    let (p, ctl) = telecharge(&url, &d, &opts(4));
    verifie(&p, &data);
    assert!(!ctl.snapshot().ranges);
}

#[test]
fn cookie_et_referer_apres_redirection() {
    // Audit M4 : cookie gardé sur le même hôte, jamais envoyé à un autre ; Referer réduit.
    let (c, url, data) = serveur(1 << 20, |_| {});
    let d = dossier("cookies");
    let mut o = opts(4);
    o.headers = vec![
        ("Cookie".into(), "session=S1".into()),
        ("Referer".into(), "http://page.example/a/b?q=1".into()),
    ];
    let (p, _) = telecharge(&url.replace("/file.bin", "/redirect"), &d, &o);
    verifie(&p, &data);
    let vus = c.vus.lock().unwrap().clone();
    let vers_fichier = |h: &str| {
        vus.iter()
            .filter(|l| l.starts_with("/file.bin | ") && l.ends_with(h))
            .count()
    };
    let requetes = vus
        .iter()
        .filter(|l| l.starts_with("/file.bin | Host"))
        .count();
    assert!(requetes >= 4, "{requetes}");
    assert_eq!(vers_fichier("Cookie: session=S1"), requetes);
    assert_eq!(vers_fichier("Referer: http://page.example/"), requetes);
    // URL finale mémorisée : une seule redirection (la sonde).
    assert_eq!(
        vus.iter()
            .filter(|l| l.starts_with("/redirect | Host"))
            .count(),
        1
    );
    // Autre hôte : pas de cookie.
    let (autre, url2, data2) = serveur(1 << 20, |_| {});
    *c.ailleurs.lock().unwrap() = url2
        .replace("/file.bin", "")
        .replace("127.0.0.1", "localhost");
    let (p, _) = telecharge(&url.replace("/file.bin", "/redirect_ext"), &d, &o);
    verifie(&p, &data2);
    let vus = autre.vus.lock().unwrap().clone();
    assert!(vus
        .iter()
        .any(|l| l.ends_with("Referer: http://page.example/")));
    assert!(!vus.iter().any(|l| l.contains("session=S1")), "{vus:?}");
}

#[test]
fn lien_expire() {
    // Audit M5 : 403 partout après une sonde réussie = lien expiré, pas « connexions en trop ».
    let (_c, url, _) = serveur(2 << 20, |c| {
        c.expire_after = 3;
        c.rate = 4 << 20;
    });
    let d = dossier("expire");
    let ctl = Arc::new(Control::default());
    let e = engine::run(&url, &d, None, &opts(4), &ctl).unwrap_err();
    assert_eq!(e, engine::ERR_EXPIRE);
    assert!(ctl.snapshot().workers.iter().all(|w| !w.dropped));
}

#[test]
fn etag_faible_ou_variable() {
    // Audit M6 : `W/` et ETag propre à un nœud de CDN (même date) ne font pas échouer.
    let (_c, url, data) = serveur(1 << 20, |c| c.etag_weak = true);
    let d = dossier("etag_faible");
    let (p, ctl) = telecharge(&url, &d, &opts(4));
    verifie(&p, &data);
    assert!(ctl.snapshot().ranges);
    let (_c, url, data) = serveur(1 << 20, |c| c.etag_varie = true);
    let (p, ctl) = telecharge(&url, &d, &opts(4));
    verifie(&p, &data);
    assert!(ctl.snapshot().ranges);
    // Sans date pour confirmer : repli sur une connexion, fichier exact.
    let (_c, url, data) = serveur(1 << 20, |c| {
        c.etag_varie = true;
        c.no_lm = true;
    });
    let (p, ctl) = telecharge(&url, &d, &opts(4));
    verifie(&p, &data);
    assert!(!ctl.snapshot().ranges);
}

#[test]
fn fichier_vide() {
    // Audit L1 : 416 + `bytes */0` = fichier de 0 octet, pas une erreur.
    let (_c, url, _) = serveur(0, |_| {});
    let d = dossier("vide");
    let (p, _) = telecharge(&url, &d, &opts(4));
    assert_eq!(std::fs::metadata(&p).unwrap().len(), 0);
    assert!(!engine::part_path(&p).exists());
}

#[test]
fn renommage_sans_ecraser() {
    // Audit L4 : un fichier apparu sous le même nom pendant le téléchargement est gardé.
    let (_c, url, data) = serveur(512 << 10, |c| c.rate = 1 << 20);
    let d = dossier("sans_ecraser");
    let ctl = Arc::new(Control::default());
    let r = std::thread::scope(|s| {
        let f = s.spawn(|| engine::run(&url, &d, None, &opts(1), &ctl));
        while !engine::part_path(&d.join("file.bin")).exists() {
            std::thread::sleep(Duration::from_millis(5));
        }
        std::fs::write(d.join("file.bin"), b"a moi").unwrap();
        f.join().unwrap()
    });
    let Ok(Outcome::Done(p)) = r else {
        panic!("{r:?}")
    };
    assert_eq!(p.file_name().unwrap(), "file (1).bin");
    assert_eq!(ctl.snapshot().file_name.as_deref(), Some("file (1).bin"));
    verifie(&p, &data);
    assert_eq!(std::fs::read(d.join("file.bin")).unwrap(), b"a moi");
}

#[test]
fn requetes_bornees_et_plages_exigees() {
    // 0.2.8 : CDN vidéo qui refuse les plages de plus de 256 Kio : chaque requête est bornée, la
    // connexion enchaîne les requêtes sur son morceau.
    let (c, url, data) = serveur(3 << 20, |c| c.max_plage = 256 << 10);
    let d = dossier("bornees");
    let mut o = opts(4);
    o.max_request = 256 << 10;
    o.require_ranges = true;
    let (p, ctl) = telecharge(&url, &d, &o);
    verifie(&p, &data);
    assert!(ctl.snapshot().ranges);
    assert!(c.requests.load(Ordering::SeqCst) >= 12);
    // Plages refusées ou ignorées : erreur claire, pas de repli sur une connexion.
    for f in [|c: &mut Cfg| c.ranges = false, |c: &mut Cfg| c.lie = true] {
        let (_c, url, _) = serveur(1 << 20, f);
        let e = engine::run(&url, &d, None, &o, &Arc::new(Control::default())).unwrap_err();
        assert_eq!(e, engine::ERR_PLAGES);
    }
}
