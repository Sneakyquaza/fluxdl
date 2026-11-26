//! Torrents et liens magnet par librqbit (Apache-2.0) : une session pour toute l'appli sur un
//! runtime tokio à part, créée au premier torrent. Le `.torrent` est gardé dans le dossier de
//! données (`torrents/<hash>.torrent`) : la reprise après un redémarrage revérifie les pièces
//! déjà sur le disque. Windows : association facultative des liens magnet et des .torrent (HKCU).

use dlcore::engine::{Control, Outcome, Segment, WorkerStat};
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, Api, ListenerOptions, Session,
    SessionOptions,
};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// Cases de la carte des pièces dessinée dans le panneau de détail.
pub const CASES: usize = 160;

pub struct Moteur {
    rt: tokio::runtime::Runtime,
    session: Arc<Session>,
    api: Api,
}

/// Fichier dans un torrent.
#[derive(Clone, Debug, PartialEq)]
pub struct Fichier {
    pub nom: String,
    pub taille: u64,
}

/// Ce qu'un torrent contient, avant de le lancer.
#[derive(Clone, Debug)]
pub struct Apercu {
    pub nom: String,
    pub hash: String,
    pub fichiers: Vec<Fichier>,
    pub octets: Vec<u8>,
}

/// Partage après la fin : arrêt quand envoyé ≥ ratio × taille (0 = pas de partage).
#[derive(Clone, Copy, Debug)]
pub struct Partage {
    pub ratio: f32,
}

fn err(e: impl std::fmt::Display) -> String {
    let s = e.to_string();
    s.chars().take(300).collect()
}

static MOTEUR: OnceLock<Result<Moteur, String>> = OnceLock::new();

/// Session de l'appli : DHT (état dans le dossier de données), trackers, écoute TCP.
pub fn moteur() -> Result<&'static Moteur, String> {
    MOTEUR
        .get_or_init(|| {
            let d = crate::store::data_dir();
            let mut dht = librqbit::DhtSessionConfig::default();
            dht.persistence = Some(librqbit::dht::DhtPersistenceConfig {
                dump_interval: None,
                config_filename: Some(d.join("dht.json")),
            });
            Moteur::new(SessionOptions {
                dht: Some(dht),
                listen: Some(ListenerOptions::default()),
                ..Default::default()
            })
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// Session déjà créée (sans la créer).
pub fn lance() -> Option<&'static Moteur> {
    MOTEUR.get().and_then(|m| m.as_ref().ok())
}

impl Moteur {
    pub fn new(opts: SessionOptions) -> Result<Moteur, String> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(err)?;
        let session = rt
            .block_on(Session::new_with_opts(std::env::temp_dir(), opts))
            .map_err(err)?;
        let api = Api::new(session.clone(), None);
        Ok(Moteur { rt, session, api })
    }

    #[cfg(test)]
    pub fn adresse(&self) -> Option<SocketAddr> {
        self.session.listen_addr()
    }

    /// Lit un lien magnet (métadonnées demandées aux pairs, 90 s au plus) ou un .torrent.
    pub fn apercu(&self, source: &str, octets: Option<Vec<u8>>) -> Result<Apercu, String> {
        let a = match octets {
            Some(o) => AddTorrent::from_bytes(o),
            None => AddTorrent::from_url(source.to_string()),
        };
        let opts = AddTorrentOptions {
            list_only: true,
            ..Default::default()
        };
        let r = self.rt.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(90),
                self.session.add_torrent(a, Some(opts)),
            )
            .await
        });
        let r = r
            .map_err(|_| "No peer answered: torrent info not found.".to_string())?
            .map_err(err)?;
        let AddTorrentResponse::ListOnly(l) = r else {
            return Err("unexpected answer".into());
        };
        let fichiers = l
            .info
            .iter_file_details()
            .map(|f| Fichier {
                nom: f.filename.to_vec().join("/"),
                taille: f.len,
            })
            .collect();
        Ok(Apercu {
            nom: l
                .info
                .name()
                .map_or_else(|| "torrent".into(), |n| n.into_owned()),
            hash: l.info_hash.as_string(),
            fichiers,
            octets: l.torrent_bytes.to_vec(),
        })
    }

    /// Télécharge (ou reprend) le torrent dans `dir`. `reprise` : fichiers déjà commencés par
    /// nous, réécrits seulement là où une pièce manque (sinon un fichier existant fait échouer
    /// l'ajout au lieu d'être écrasé). Arrêt = pause dans la session (reprise immédiate).
    #[allow(clippy::too_many_arguments)]
    pub fn telecharger(
        &self,
        octets: Vec<u8>,
        dir: &Path,
        fichiers: Option<Vec<usize>>,
        reprise: bool,
        partage: Partage,
        pairs: Option<Vec<SocketAddr>>,
        ctl: &Control,
    ) -> Result<Outcome, String> {
        // Plusieurs fichiers : dans un sous-dossier au nom du torrent (librqbit écrit sinon
        // directement dans le dossier donné).
        let (nom, multi) = {
            let m = librqbit::torrent_from_bytes(&octets).map_err(err)?;
            let i = &m.info.data;
            let nom = i
                .name
                .as_ref()
                .map(|n| dlcore::engine::sanitize(&String::from_utf8_lossy(n.as_ref())))
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| "torrent".into());
            (nom, i.files.is_some())
        };
        let cible = dir.join(&nom);
        let sortie = if multi {
            cible.clone()
        } else {
            dir.to_path_buf()
        };
        std::fs::create_dir_all(&sortie).map_err(|e| format!("{}: {e}", sortie.display()))?;
        let opts = AddTorrentOptions {
            output_folder: Some(sortie.display().to_string()),
            only_files: fichiers,
            overwrite: reprise,
            initial_peers: pairs,
            ..Default::default()
        };
        let r = self
            .rt
            .block_on(
                self.session
                    .add_torrent(AddTorrent::from_bytes(octets), Some(opts)),
            )
            .map_err(err)?;
        let h = match r {
            AddTorrentResponse::Added(_, h) => h,
            AddTorrentResponse::AlreadyManaged(_, h) => {
                if h.is_paused() {
                    self.rt.block_on(self.session.unpause(&h)).map_err(err)?;
                }
                h
            }
            AddTorrentResponse::ListOnly(_) => return Err("unexpected answer".into()),
        };
        ctl.lock().workers = vec![WorkerStat::default()];
        loop {
            let s = h.stats();
            if let Some(e) = &s.error {
                return Err(err(e));
            }
            let haves = self.api.api_dump_haves(h.id().into()).ok();
            {
                let mut l = ctl.lock();
                l.file_name = h.name();
                l.size = Some(s.total_bytes);
                l.received = s.progress_bytes;
                l.uploaded = s.uploaded_bytes;
                l.segments = vec![Segment {
                    start: 0,
                    pos: s.progress_bytes,
                    end: s.total_bytes,
                    worker: Some(0),
                    stolen: false,
                }];
                if let Some(v) = &s.live {
                    l.peers = v.snapshot.peer_stats.live;
                    l.up_speed = v.upload_speed.mbps * 1_048_576.0;
                }
                if let Some((bf, n)) = haves {
                    l.pieces = carte(|i| bf.get(i).is_some_and(|b| *b), n as usize);
                }
                l.seeding = s.finished;
            }
            let assez = s.uploaded_bytes as f64 >= partage.ratio as f64 * s.total_bytes as f64;
            let stop = ctl.stop.load(Ordering::Relaxed);
            if (s.finished && (partage.ratio <= 0.0 || assez)) || stop {
                let _ = self.rt.block_on(self.session.pause(&h));
                return Ok(if s.finished {
                    ctl.lock().seeding = false;
                    Outcome::Done(cible)
                } else {
                    Outcome::Stopped
                });
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// Retire le torrent de la session ; `fichiers` : efface aussi ce qu'il a écrit.
    pub fn oublier(&self, hash: &str, fichiers: bool) {
        if let Ok(id) = librqbit::dht::Id20::from_str(hash) {
            let _ = self.rt.block_on(self.session.delete(id.into(), fichiers));
        }
    }
}

/// Carte réduite : `CASES` cases, chacune = part des pièces reçues (0 à 255).
pub fn carte(a: impl Fn(usize) -> bool, n: usize) -> Vec<u8> {
    if n == 0 {
        return Vec::new();
    }
    let k = CASES.min(n);
    (0..k)
        .map(|c| {
            let (d, f) = (c * n / k, ((c + 1) * n / k).max(c * n / k + 1));
            let recues = (d..f).filter(|&i| a(i)).count();
            (recues * 255 / (f - d)) as u8
        })
        .collect()
}

/// Dossier des `.torrent` gardés pour la reprise.
pub fn dossier() -> PathBuf {
    crate::store::data_dir().join("torrents")
}

/// Chemin du .torrent gardé pour ce hash.
pub fn garde(hash: &str) -> PathBuf {
    dossier().join(format!("{hash}.torrent"))
}

/// Windows : liens magnet et fichiers .torrent ouverts par fluxdl (HKCU, sans droits admin).
/// Désactiver retire seulement ce qui pointe vers nous.
#[cfg(windows)]
pub fn associer(oui: bool) -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let cmd = format!("\"{}\" \"%1\"", exe.display());
    let reg = |args: &[&str]| {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("reg")
            .args(args)
            .creation_flags(0x0800_0000)
            .output()
            .ok()
    };
    let ok = |args: &[&str]| reg(args).is_some_and(|o| o.status.success());
    const C: &str = r"HKCU\Software\Classes";
    let (magnet, progid) = (format!(r"{C}\magnet"), format!(r"{C}\fluxdl.torrent"));
    if oui {
        let ouvre = |k: &str| {
            ok(&[
                "add",
                &format!(r"{k}\shell\open\command"),
                "/ve",
                "/d",
                &cmd,
                "/f",
            ])
        };
        ok(&["add", &magnet, "/ve", "/d", "URL:Magnet link", "/f"])
            && ok(&["add", &magnet, "/v", "URL Protocol", "/d", "", "/f"])
            && ouvre(&magnet)
            && ok(&["add", &progid, "/ve", "/d", "Torrent", "/f"])
            && ok(&[
                "add",
                &format!(r"{progid}\DefaultIcon"),
                "/ve",
                "/d",
                &format!("\"{}\",0", exe.display()),
                "/f",
            ])
            && ouvre(&progid)
            && ok(&[
                "add",
                &format!(r"{C}\.torrent"),
                "/ve",
                "/d",
                "fluxdl.torrent",
                "/f",
            ])
    } else {
        let a_nous = |k: &str| {
            reg(&["query", &format!(r"{k}\shell\open\command"), "/ve"]).is_some_and(|o| {
                String::from_utf8_lossy(&o.stdout).contains(&exe.display().to_string())
            })
        };
        if a_nous(&magnet) {
            ok(&["delete", &magnet, "/f"]);
        }
        let ext = reg(&["query", &format!(r"{C}\.torrent"), "/ve"]);
        if ext.is_some_and(|o| String::from_utf8_lossy(&o.stdout).contains("fluxdl.torrent")) {
            ok(&["delete", &format!(r"{C}\.torrent"), "/f"]);
        }
        ok(&["delete", &progid, "/f"]);
        true
    }
}

#[cfg(not(windows))]
pub fn associer(_oui: bool) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use librqbit::{create_torrent, CreateTorrentOptions};

    /// Session isolée : ni DHT, ni trackers, ni découverte locale ; écoute sur 127.0.0.1.
    fn moteur_local() -> Moteur {
        Moteur::new(SessionOptions {
            dht: None,
            disable_trackers: true,
            disable_local_service_discovery: true,
            listen: Some(ListenerOptions {
                listen_addr: "127.0.0.1:0".parse().unwrap(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .unwrap()
    }

    fn dossier_test(nom: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fluxdl-test-tor-{nom}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn carte_des_pieces() {
        assert!(carte(|_| true, 0).is_empty());
        assert_eq!(carte(|i| i < 2, 4), vec![255, 255, 0, 0]);
        let c = carte(|i| i % 2 == 0, 1000);
        assert_eq!(c.len(), CASES);
        assert!(c.iter().all(|&v| (100..=155).contains(&v)));
    }

    #[test]
    fn ajout_echange_local_et_reprise() {
        // Petit torrent créé sur place (2 fichiers, pièces de 16 Kio) : aucun réseau extérieur.
        let src = dossier_test("src");
        let contenu = dossier_test("contenu").join("jeu");
        std::fs::create_dir_all(&contenu).unwrap();
        let a: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect();
        std::fs::write(contenu.join("a.bin"), &a).unwrap();
        std::fs::write(contenu.join("b.txt"), b"bonjour").unwrap();
        let semeur = moteur_local();
        let octets = semeur
            .rt
            .block_on(async {
                let sp = librqbit::spawn_utils::BlockingSpawner::new(1);
                create_torrent(
                    &contenu,
                    CreateTorrentOptions {
                        name: Some("jeu"),
                        trackers: vec![],
                        piece_length: Some(16384),
                    },
                    &sp,
                )
                .await
            })
            .unwrap()
            .as_bytes()
            .unwrap()
            .to_vec();
        let ap = semeur.apercu("", Some(octets.clone())).unwrap();

        assert_eq!(ap.nom, "jeu");
        let mut f = ap.fichiers.clone();
        f.sort_by(|x, y| x.nom.cmp(&y.nom));
        assert_eq!(
            f,
            vec![
                Fichier {
                    nom: "a.bin".into(),
                    taille: 300_000
                },
                Fichier {
                    nom: "b.txt".into(),
                    taille: 7
                }
            ]
        );
        // Semeur : les fichiers sont déjà complets → fini sans aucun pair (pièces revérifiées).
        let parent = contenu.parent().unwrap().to_path_buf();
        let ctl = Control::default();
        let p = Partage { ratio: 0.0 };
        let r = semeur
            .telecharger(octets.clone(), &parent, None, true, p, None, &ctl)
            .unwrap();
        assert!(matches!(r, Outcome::Done(_)));
        assert_eq!(
            ctl.snapshot().pieces.iter().filter(|&&v| v == 255).count(),
            ctl.snapshot().pieces.len()
        );
        // Le semeur repart pour partager ; un 2e client télécharge chez lui seul (127.0.0.1).
        let s_ctl = Arc::new(Control::default());
        let s2 = s_ctl.clone();
        let sm: &'static Moteur = Box::leak(Box::new(semeur));
        let o2 = octets.clone();
        let fil = std::thread::spawn(move || {
            sm.telecharger(
                o2,
                &parent,
                None,
                true,
                Partage { ratio: 1000.0 },
                None,
                &s2,
            )
        });
        let client = moteur_local();
        let dest = dossier_test("dest");
        let ctl = Control::default();
        let pairs = Some(vec![sm.adresse().unwrap()]);
        let fin = std::thread::scope(|s| {
            let f =
                s.spawn(|| client.telecharger(octets.clone(), &dest, None, false, p, pairs, &ctl));
            let t = std::time::Instant::now();
            while !f.is_finished() && t.elapsed() < Duration::from_secs(30) {
                std::thread::sleep(Duration::from_millis(50));
            }
            ctl.request_stop();
            f.join().unwrap()
        });
        s_ctl.request_stop();
        let _ = fil.join();
        assert!(matches!(fin, Ok(Outcome::Done(_))), "{:?}", fin.err());
        assert_eq!(std::fs::read(dest.join("jeu").join("a.bin")).unwrap(), a);
        assert_eq!(
            std::fs::read(dest.join("jeu").join("b.txt")).unwrap(),
            b"bonjour"
        );
        // Un fichier existant qui n'est pas à nous : refusé, jamais écrasé.
        let autre = dossier_test("autre");
        std::fs::create_dir_all(autre.join("jeu")).unwrap();
        std::fs::write(autre.join("jeu").join("b.txt"), b"a moi").unwrap();
        let ctl = Control::default();
        let r = moteur_local().telecharger(octets, &autre, None, false, p, None, &ctl);
        assert!(r.is_err());
        assert_eq!(
            std::fs::read(autre.join("jeu").join("b.txt")).unwrap(),
            b"a moi"
        );
        for d in [src, contenu.parent().unwrap().to_path_buf(), dest, autre] {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}
