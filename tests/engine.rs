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
    let n_req = c.requests.fetch_add(1, Ordering::SeqCst);
    let data = c.data.lock().unwrap().clone();
    let etag = c.etag.lock().unwrap().clone();
    let total = data.len() as u64;
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
    let plage = plage.filter(|_| !(c.lie && n_req > 0));
    let mut tete = String::new();
    let (a, b) = match plage {
        Some((a, b)) => {
            tete += &format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {a}-{b}/{total}\r\n"
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
    tete += &format!("ETag: \"{etag}\"\r\nLast-Modified: Wed, 25 Nov 2026 10:00:00 GMT\r\nConnection: close\r\n\r\n");
    s.write_all(tete.as_bytes())?;
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
        s.write_all(&data[p as usize..q as usize])?;
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

fn telecharge(url: &str, d: &Path, o: &Options) -> (PathBuf, Control) {
    let ctl = Control::default();
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
    let e = engine::run(&url, &d, None, &o, &Control::default()).unwrap_err();
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
    let ctl = Control::default();
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
    let ctl2 = Control::default();
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
    let ctl = Control::default();
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
    let ctl = Control::default();
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
