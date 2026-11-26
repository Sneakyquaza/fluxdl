//! Fenêtre egui : barre latérale, file de téléchargements, panneau de détail par connexion,
//! comparatif, réglages.

mod draw;
mod fenetres;
mod guide;

use crate::i18n::t;
use crate::maj::{self, Maj};
use crate::serveur::{self, Demande};
use crate::store::{self, Entry, Kind, Lang, Settings, Status};
use crate::{torrent, video};
use dlcore::engine::{self, Control, Live, Options, Outcome, Segment, WorkerStat};
use dlcore::APP_NAME;
use draw::*;
use eframe::egui::{
    self, pos2, vec2, Align, Align2, Color32, Layout, Margin, Rect, Response, Sense, Stroke,
    StrokeKind, Ui, UiBuilder, Vec2,
};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Téléchargements simultanés au plus ; les suivants attendent.
const MAX_ACTIFS: usize = 3;
const QUEUE: &str = "queue.json";
const SETTINGS: &str = "settings.json";
/// Carte d'astuces de la barre latérale : (icône, titre, texte), une toutes les `ASTUCE_S` s.
const ASTUCES: [(Icone, &str, &str); 8] = [
    (
        Icone::Eclair,
        "Done early? Take half.",
        "A connection that finishes early takes half of the largest remaining range.",
    ),
    (
        Icone::Actualiser,
        "Picks up where it stopped",
        "After a crash, a power cut or a pause, a download resumes from where it was.",
    ),
    (
        Icone::Lien,
        "Browser extension",
        "Pair it in Settings, turn on auto-capture: browser downloads land here.",
    ),
    (
        Icone::Lecture,
        "Videos too",
        "Paste a video page link: yt-dlp fetches it in the quality you pick, or audio only.",
    ),
    (
        Icone::Aimant,
        "Magnets and torrents",
        "Paste a magnet link or drop a .torrent file onto the window.",
    ),
    (
        Icone::Comparatif,
        "Measure the gain",
        "The Benchmark tab downloads the same link with 1 connection, then with N.",
    ),
    (
        Icone::Reglages,
        "Leave some bandwidth",
        "Settings → Speed limit: each download leaves bandwidth for everyone else.",
    ),
    (
        Icone::Telecharger,
        "Always up to date",
        "fluxdl tells you when a new version is out and updates itself in one click.",
    ),
];
const ASTUCE_S: f64 = 12.0;
const FONDU_S: f64 = 0.6;

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Downloads,
    Benchmark,
    Settings,
}

/// Téléchargement suivi (en cours ou fini pendant cette session).
struct Job {
    ctl: Arc<Control>,
    handle: Option<JoinHandle<Result<Outcome, String>>>,
    /// Débit total lissé et par connexion (octets/s).
    speed: f64,
    speeds: Vec<f64>,
    history: VecDeque<f32>,
    last: (f64, u64, Vec<u64>),
}

impl Job {
    fn new(ctl: Arc<Control>, handle: Option<JoinHandle<Result<Outcome, String>>>) -> Self {
        Job {
            ctl,
            handle,
            speed: 0.0,
            speeds: Vec::new(),
            history: VecDeque::new(),
            last: (0.0, 0, Vec::new()),
        }
    }

    /// Mesure du débit toutes les 0,5 s.
    fn sample(&mut self, now: f64, l: &Live) {
        let dt = now - self.last.0;
        if dt < 0.5 {
            return;
        }
        let par: Vec<u64> = l.workers.iter().map(|w| w.bytes).collect();
        if self.last.0 > 0.0 && l.received >= self.last.1 {
            let v = (l.received - self.last.1) as f64 / dt;
            self.speed = if self.speed == 0.0 {
                v
            } else {
                0.6 * v + 0.4 * self.speed
            };
            self.speeds = par
                .iter()
                .enumerate()
                .map(|(i, b)| b.saturating_sub(*self.last.2.get(i).unwrap_or(b)) as f64 / dt)
                .collect();
            self.history.push_back(self.speed as f32);
            if self.history.len() > 120 {
                self.history.pop_front();
            }
        }
        self.last = (now, l.received, par);
    }
}

/// Résultat attendu d'un fil (infos vidéo, aperçu de torrent…).
type Attente<T> = Option<Receiver<T>>;

/// yt-dlp / ffmpeg à télécharger (Windows) : lecture des releases, accord, installation.
enum EtatOutils {
    Lecture(Receiver<Result<video::Paquet, &'static str>>),
    Pret(video::Paquet),
    /// Reçus / total, fin, arrêt demandé.
    Installation(
        Arc<Mutex<(u64, u64)>>,
        Receiver<Result<(), &'static str>>,
        Arc<AtomicBool>,
    ),
    Erreur(&'static str),
}

/// Fenêtre modale par-dessus l'appli.
enum Modale {
    /// Mise à jour demandée pendant des téléchargements : confirmation.
    MajActifs(usize),
    /// Choix du format d'une vidéo (formats lus par yt-dlp dans un fil).
    Video {
        url: String,
        rx: Attente<Result<video::Infos, String>>,
        infos: Option<Result<video::Infos, String>>,
        choix: video::Qualite,
    },
    /// Outils vidéo manquants ; `url` : vidéo à ouvrir ensuite (vide depuis les réglages).
    Outils { url: String, etat: EtatOutils },
    /// Fichiers d'un torrent (lien magnet résolu par les pairs dans un fil).
    Torrent {
        source: String,
        rx: Attente<Result<torrent::Apercu, String>>,
        apercu: Option<Result<torrent::Apercu, String>>,
        coches: Vec<bool>,
    },
    /// Premier torrent (Windows) : ouvrir les liens magnet et .torrent avec l'appli ?
    Association,
    /// Le lien mène à une page web, pas à un fichier : l'ouvrir dans le navigateur.
    Page(String),
    /// Annuler un téléchargement : effacer aussi le fichier partiel ? (oui par défaut)
    Annuler { id: u64, supprimer: bool },
    /// Installation guidée de l'extension.
    Guide(guide::Guide),
    /// Une extension demande l'appairage : « Autoriser ? ».
    Appairage {
        depuis: std::time::Instant,
        origine: String,
        navigateur: &'static str,
    },
}

/// Phase du comparatif (1 = une connexion, 2 = N) et son contrôle.
type Phase = (u8, Arc<Control>);

#[derive(Default)]
struct Bench {
    url: String,
    connections: usize,
    secs: u32,
    handle: Option<JoinHandle<Result<(f64, f64), String>>>,
    /// Phase en cours (1 = une connexion, 2 = N) et son contrôle, pour l'affichage en direct.
    live: Arc<Mutex<Option<Phase>>>,
    cancel: Arc<AtomicBool>,
    result: Option<Result<(f64, f64), String>>,
    /// Instant d'arrivée du résultat (barres qui poussent).
    shown: f64,
}

pub struct App {
    settings: Settings,
    entries: Vec<Entry>,
    jobs: HashMap<u64, Job>,
    /// Annulés, déjà retirés de la file, dont le fil finit : (job, entrée, effacer les partiels).
    orphelins: Vec<(Job, Entry, bool)>,
    tab: Tab,
    url: String,
    sha: String,
    selected: Option<u64>,
    bench: Bench,
    next_id: u64,
    last_save: f64,
    capture: Option<(PathBuf, u32)>,
    /// Textures à garder en vie : style (profils, fond) et logo.
    _textures: Vec<egui::TextureHandle>,
    logo: egui::TextureHandle,
    now: f64,
    /// Instant d'ajout de chaque nouveau téléchargement (apparition en fondu).
    born: HashMap<u64, f64>,
    /// Carte d'astuces : (actuelle, précédente, depuis quand).
    astuce: (usize, usize, f64),
    /// Sélecteur de dossier ouvert dans un fil.
    picker: Option<Receiver<String>>,
    /// Sélecteur de fichier .torrent ouvert dans un fil.
    picker_torrent: Option<Receiver<String>>,
    maj: Maj,
    /// Prochaine recherche silencieuse de mise à jour (toutes les 6 h).
    maj_prochaine: f64,
    /// Version écartée par « Plus tard » (bulle cachée jusqu'à la suivante).
    maj_cachee: Option<String>,
    /// Installer et redémarrer dès que la nouvelle version est prête (accord donné).
    maj_auto: bool,
    /// Exécutable à relancer après la fermeture (mise à jour installée).
    relance: Option<PathBuf>,
    modale: Option<Modale>,
    /// Liens envoyés par l'extension (serveur local) ou par une 2e instance.
    demandes: Option<Receiver<Demande>>,
    jeton: Arc<Mutex<String>>,
    /// Demande d'appairage automatique (partagée avec le serveur).
    appairage: Arc<Mutex<serveur::Appairage>>,
    serveur_err: Option<String>,
    outils: video::Outils,
    /// Captures : défilement imposé de la page des réglages.
    defile: Option<f32>,
    /// Liens copiés ailleurs (fil de `presse`) ; surveillance active ou non.
    presse: Option<Receiver<String>>,
    presse_actif: Arc<AtomicBool>,
    /// Dernier texte copié par l'appli elle-même (jamais proposé).
    copie_app: Option<String>,
    /// Bulle « Lien copié » : lien et instant d'apparition.
    lien_copie: Option<(String, f64)>,
}

impl App {
    /// `ouvrir` : lien magnet ou .torrent passé en ligne de commande (association Windows).
    pub fn new(ctx: &egui::Context, capture: Option<PathBuf>, ouvrir: Option<String>) -> Self {
        let textures = installer_style(ctx);
        let logo = ctx.load_texture(
            "logo",
            egui::ColorImage::from_rgba_unmultiplied([64, 64], &crate::icone::icone_rgba(64)),
            egui::TextureOptions {
                mipmap_mode: Some(egui::TextureFilter::Linear),
                ..egui::TextureOptions::LINEAR
            },
        );
        let mut app = App {
            settings: Settings::default(),
            entries: Vec::new(),
            jobs: HashMap::new(),
            orphelins: Vec::new(),
            tab: Tab::Downloads,
            url: String::new(),
            sha: String::new(),
            selected: None,
            bench: Bench {
                connections: 16,
                secs: 8,
                ..Default::default()
            },
            next_id: 1,
            last_save: 0.0,
            capture: None,
            _textures: textures,
            logo,
            now: 0.0,
            born: HashMap::new(),
            astuce: (0, 0, 0.0),
            picker: None,
            picker_torrent: None,
            maj: Maj::new({
                let c = ctx.clone();
                move || c.request_repaint()
            }),
            maj_prochaine: 3.0,
            maj_cachee: None,
            maj_auto: false,
            relance: None,
            modale: None,
            demandes: None,
            jeton: Arc::new(Mutex::new(String::new())),
            appairage: Arc::default(),
            serveur_err: None,
            outils: video::Outils::default(),
            defile: None,
            presse: None,
            presse_actif: Arc::new(AtomicBool::new(false)),
            copie_app: None,
            lien_copie: None,
        };
        if let Some(d) = capture {
            app.demo();
            app.capture = Some((d, 0));
            return app;
        }
        if let Some(s) = store::load(SETTINGS) {
            app.settings = s;
        }
        app.entries = store::load(QUEUE).unwrap_or_default();
        app.presse_actif
            .store(app.settings.clipboard_watch, Ordering::Relaxed);
        app.presse = Some(crate::presse::lancer(ctx.clone(), app.presse_actif.clone()));
        for e in &mut app.entries {
            store::au_lancement(e);
        }
        app.next_id = app.entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        maj::nettoyer();
        crate::extension::mettre_a_jour(&crate::extension::dossier());
        app.outils = video::trouver();
        // yt-dlp suit les changements des sites : notre copie se met à jour chaque semaine.
        let maintenant = secondes();
        if maintenant.saturating_sub(app.settings.ytdlp_updated) > 7 * 86_400 {
            video::mettre_a_jour_ytdlp();
            app.settings.ytdlp_updated = maintenant;
        }
        if app.settings.token.is_empty() {
            app.settings.token = serveur::nouveau_jeton();
        }
        app.lancer_serveur(ctx);
        if let Some(u) = ouvrir {
            app.ajouter(u, None, None, true);
        }
        app
    }

    /// Serveur local de l'extension ; serveur coupé = jeton vide (tout est refusé).
    fn lancer_serveur(&mut self, ctx: &egui::Context) {
        self.sync_jeton();
        let (tx, rx) = std::sync::mpsc::channel();
        let c = ctx.clone();
        match serveur::lancer(
            self.settings.port,
            self.jeton.clone(),
            self.appairage.clone(),
            tx,
            move || c.request_repaint(),
        ) {
            Ok(_) => self.demandes = Some(rx),
            Err(e) => self.serveur_err = Some(e.to_string()),
        }
    }

    fn sync_jeton(&self) {
        *self.jeton.lock().unwrap_or_else(|e| e.into_inner()) = if self.settings.server {
            self.settings.token.clone()
        } else {
            String::new()
        };
    }

    /// Aiguillage d'un lien : magnet / .torrent → choix des fichiers, site vidéo → choix du
    /// format, sinon téléchargement multi-connexions. `local` : chemin de .torrent permis.
    fn ajouter(
        &mut self,
        url: String,
        referrer: Option<String>,
        cookies: Option<String>,
        local: bool,
    ) {
        let url = url.trim().to_string();
        if url.starts_with("magnet:") {
            return self.ouvrir_torrent(url, None);
        }
        if local && url.to_ascii_lowercase().ends_with(".torrent") {
            let nom = Path::new(&url)
                .file_name()
                .map_or(url.clone(), |n| n.to_string_lossy().into_owned());
            if let Ok(o) = std::fs::read(&url) {
                self.ouvrir_torrent(nom, Some(o));
            }
            return;
        }
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return;
        }
        if video::site_video(&url) {
            return self.ouvrir_video(url);
        }
        // Déjà dans la file : la montrer plutôt que télécharger deux fois le même fichier.
        if let Some(e) = self
            .entries
            .iter()
            .find(|e| e.url == url && !matches!(e.status, Status::Done | Status::Canceled))
        {
            self.selected = Some(e.id);
            self.tab = Tab::Downloads;
            return;
        }
        let sha = std::mem::take(&mut self.sha).trim().to_string();
        let e = self.nouvelle(url, Kind::Http);
        e.sha256 = (!sha.is_empty()).then_some(sha);
        e.referrer = referrer;
        e.cookies = cookies;
    }

    /// Ajoute une entrée en attente, choisie dans la liste.
    fn nouvelle(&mut self, url: String, kind: Kind) -> &mut Entry {
        self.entries.push(Entry {
            id: self.next_id,
            url,
            dir: self.settings.folder.clone(),
            name: None,
            sha256: None,
            connections: self.settings.connections,
            status: Status::Queued,
            size: None,
            done: 0,
            kind,
            referrer: None,
            cookies: None,
        });
        self.selected = Some(self.next_id);
        self.born.insert(self.next_id, self.now);
        self.next_id += 1;
        self.tab = Tab::Downloads;
        self.entries.last_mut().expect("entrée ajoutée")
    }

    /// Vidéo : formats lus par yt-dlp ; outils manquants → accord (Windows) ou message.
    fn ouvrir_video(&mut self, url: String) {
        if cfg!(windows) && !self.outils.complets() {
            let o = self.outils.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(video::preparer(&o));
            });
            self.modale = Some(Modale::Outils {
                url,
                etat: EtatOutils::Lecture(rx),
            });
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let o = self.outils.clone();
        let u = url.clone();
        std::thread::spawn(move || {
            let r = if o.ytdlp.is_none() {
                Err(video::ABSENT.to_string())
            } else {
                video::lire_infos(&o, &u)
            };
            let _ = tx.send(r);
        });
        self.modale = Some(Modale::Video {
            url,
            rx: Some(rx),
            infos: None,
            choix: video::Qualite::Best,
        });
    }

    /// Torrent : liste des fichiers lue dans un fil (un lien magnet interroge les pairs).
    fn ouvrir_torrent(&mut self, source: String, octets: Option<Vec<u8>>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let s = source.clone();
        std::thread::spawn(move || {
            let _ = tx.send(torrent::moteur().and_then(|m| m.apercu(&s, octets)));
        });
        self.modale = Some(Modale::Torrent {
            source,
            rx: Some(rx),
            apercu: None,
            coches: Vec::new(),
        });
    }

    fn add(&mut self) {
        let url = std::mem::take(&mut self.url);
        self.ajouter(url, None, None, true);
    }

    fn start(&mut self, i: usize) {
        let e = &mut self.entries[i];
        e.status = Status::Running;
        let ctl = Arc::new(Control::default());
        let (url, dir, name, c) = (
            e.url.clone(),
            PathBuf::from(&e.dir),
            e.name.clone(),
            ctl.clone(),
        );
        let h = match e.kind.clone() {
            Kind::Http => {
                let mut headers = Vec::new();
                if let Some(r) = &e.referrer {
                    headers.push(("Referer".to_string(), r.clone()));
                }
                if let Some(k) = &e.cookies {
                    headers.push(("Cookie".to_string(), k.clone()));
                }
                let opts = Options {
                    connections: e.connections,
                    speed_limit: (self.settings.speed_limit_mb.max(0.0) * 1e6) as u64,
                    expected_sha256: e.sha256.clone(),
                    headers,
                    ..Default::default()
                };
                std::thread::spawn(move || engine::run(&url, &dir, name.as_deref(), &opts, &c))
            }
            Kind::Video { quality, estimate } => {
                let o = self.outils.clone();
                std::thread::spawn(move || {
                    video::telecharger(&o, &url, &dir, quality, estimate, &c)
                })
            }
            Kind::Torrent {
                hash,
                files,
                started,
            } => {
                let partage = torrent::Partage {
                    ratio: self.settings.seed_ratio,
                };
                std::thread::spawn(move || {
                    let o = std::fs::read(torrent::garde(&hash)).map_err(|e| e.to_string())?;
                    torrent::moteur()?.telecharger(o, &dir, files, started, partage, None, &c)
                })
            }
        };
        self.jobs.insert(e.id, Job::new(ctl, Some(h)));
    }

    /// Fin des téléchargements terminés, démarrage des suivants, noms et tailles à jour.
    fn schedule(&mut self, now: f64) {
        let mut page = None;
        let mut fini = false;
        for e in &mut self.entries {
            let Some(j) = self.jobs.get_mut(&e.id) else {
                continue;
            };
            let l = j.ctl.snapshot();
            j.sample(now, &l);
            if l.file_name.is_some() {
                e.name = l.file_name.clone();
            }
            e.size = l.size.or(e.size);
            e.done = l.done();
            // Le torrent a démarré : ses fichiers sont à nous, la reprise peut les réécrire.
            if let Kind::Torrent { started, .. } = &mut e.kind {
                *started |= l.size.is_some();
            }
            if !j.handle.as_ref().is_some_and(|h| h.is_finished()) {
                continue;
            }
            let r = j
                .handle
                .take()
                .map(|h| h.join().unwrap_or_else(|_| Err("panic".into())));
            j.speed = 0.0;
            fini = true;
            e.status = match r {
                Some(Ok(Outcome::Done(_))) => Status::Done,
                Some(Ok(Outcome::Stopped)) => Status::Paused,
                Some(Err(m)) => {
                    if m == engine::ERR_PAGE {
                        page = Some(e.url.clone());
                    }
                    Status::Failed(m)
                }
                None => e.status.clone(),
            };
        }
        // Annulés : effacer les partiels une fois leur fil fini (il sauve encore l'état en partant).
        self.orphelins.retain(|(j, e, effacer)| {
            if j.handle.as_ref().is_some_and(|h| !h.is_finished()) {
                return true;
            }
            delete_parts(e, *effacer);
            false
        });
        // Fin, pause ou échec : la file est écrite tout de suite.
        if fini {
            self.last_save = f64::NEG_INFINITY;
        }
        // Page web au lieu d'un fichier : explication et bouton vers le navigateur.
        if let Some(url) = page.filter(|_| self.modale.is_none()) {
            self.modale = Some(Modale::Page(url));
        }
        // Un torrent qui partage après la fin ne prend pas de place dans la file.
        let mut actifs = self
            .entries
            .iter()
            .filter(|e| e.status == Status::Running && !self.partage(e.id))
            .count();
        for i in 0..self.entries.len() {
            if actifs < MAX_ACTIFS && self.entries[i].status == Status::Queued {
                self.start(i);
                actifs += 1;
            }
        }
    }

    /// Torrent fini qui partage encore.
    fn partage(&self, id: u64) -> bool {
        self.jobs.get(&id).is_some_and(|j| j.ctl.lock().seeding)
    }

    fn persist(&mut self, now: f64) {
        if self.capture.is_none() && now - self.last_save > 2.0 {
            store::save(QUEUE, &self.entries);
            store::save(SETTINGS, &self.settings);
            self.last_save = now;
        }
    }

    /// Barre latérale : logo, navigation (indicateur qui glisse), rappel du principe.
    fn sidebar(&mut self, ui: &mut Ui) {
        let l = self.settings.lang;
        // Le principe du vol de travail, en bas : c'est l'identité de l'appli.
        egui::Panel::bottom("pied")
            .frame(egui::Frame::NONE)
            .resizable(false)
            .show_separator_line(false)
            .show(ui, |ui| {
                // Astuce suivante toutes les 12 s (fondu enchaîné), ou au clic.
                let (mut cur, mut avant, mut depuis) = self.astuce;
                if self.capture.is_none() && self.now - depuis >= ASTUCE_S {
                    (avant, cur, depuis) = (cur, (cur + 1) % ASTUCES.len(), self.now);
                }
                let a = ((self.now - depuis) / FONDU_S).clamp(0.0, 1.0) as f32;
                let fond = ui.painter().add(egui::Shape::Noop);
                let cadre = egui::Frame::NONE
                    .inner_margin(Margin::same(14))
                    .show(ui, |ui| {
                        let w = ui.available_width();
                        let p = ui.painter().clone();
                        let textes = |k: usize| {
                            let (_, titre, texte) = ASTUCES[k];
                            (
                                p.layout(t(l, titre).into(), demi(13.5), TITRE, w),
                                p.layout(t(l, texte).into(), regulier(12.5), TEXTE2, w),
                            )
                        };
                        // Hauteur fixe (la plus longue astuce) : la carte ne saute pas.
                        let h = (0..ASTUCES.len())
                            .map(|k| {
                                let (g1, g2) = textes(k);
                                g1.size().y + g2.size().y
                            })
                            .fold(0.0, f32::max);
                        let (r, _) =
                            ui.allocate_exact_size(vec2(w, 26.0 + 6.0 + h), Sense::hover());
                        let a = if cur == avant { 1.0 } else { a };
                        for (k, o) in [(avant, 1.0 - a), (cur, a)] {
                            if o <= 0.0 {
                                continue;
                            }
                            let mut p = ui.painter().clone();
                            p.multiply_opacity(o);
                            let (ic, _, _) = ASTUCES[k];
                            let gel = if k % 2 == 0 { GEL_VERT } else { GEL_BLEU };
                            tuile_icone(&p, Rect::from_min_size(r.min, Vec2::splat(26.0)), ic, gel);
                            let (g1, g2) = textes(k);
                            let y = r.top() + 32.0 + g1.size().y;
                            p.galley(pos2(r.left(), r.top() + 32.0), g1, TITRE);
                            p.galley(pos2(r.left(), y), g2, TEXTE2);
                        }
                        // Points : un par astuce, l'actuelle en bleu.
                        let n = ASTUCES.len() as f32;
                        for k in 0..ASTUCES.len() {
                            let c = pos2(
                                r.right() - 3.0 - (n - 1.0 - k as f32) * 10.0,
                                r.top() + 13.0,
                            );
                            let col = if k == cur {
                                mix(nuit(50), TITRE, a)
                            } else if k == avant {
                                mix(TITRE, nuit(50), a)
                            } else {
                                nuit(50)
                            };
                            ui.painter().circle_filled(c, 3.0, col);
                        }
                    });
                // Encart : verre plus clair, posé dans le verre de la barre.
                let r = cadre.response.rect;
                ui.painter().set(
                    fond,
                    egui::Shape::Vec(vec![
                        vdeg(r, 12, blanc(215), blanc(150)),
                        reflet(r.shrink(1.0), 11, 0.5),
                        egui::Shape::rect_stroke(
                            r,
                            12,
                            Stroke::new(1.0, nuit(45)),
                            StrokeKind::Inside,
                        ),
                    ]),
                );
                if ui
                    .interact(r, egui::Id::unique("astuce"), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    (avant, cur, depuis) = (cur, (cur + 1) % ASTUCES.len(), self.now);
                }
                self.astuce = (cur, avant, depuis);
                // Images seulement pendant le fondu (30 i/s) ; sinon une seule, au prochain
                // changement.
                if self.capture.is_none() {
                    let reste = (self.now - depuis).max(0.0);
                    if reste < FONDU_S {
                        ui.ctx().request_repaint_after(Duration::from_millis(33));
                    } else {
                        ui.ctx()
                            .request_repaint_after(Duration::from_secs_f64(ASTUCE_S - reste));
                    }
                }
                ui.add_space(14.0);
                ui.label(
                    egui::RichText::new(format!(
                        "{} {}",
                        t(l, "Version"),
                        env!("CARGO_PKG_VERSION")
                    ))
                    .font(regulier(11.5))
                    .color(TEXTE3),
                );
            });
        let w = ui.available_width();
        let (r, _) = ui.allocate_exact_size(vec2(w, 32.0), Sense::hover());
        let logo =
            Rect::from_min_size(pos2(r.left() + 6.0, r.center().y - 15.0), Vec2::splat(30.0));
        ui.painter().image(
            self.logo.id(),
            logo,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        texte(
            ui,
            pos2(logo.right() + 12.0, r.center().y),
            Align2::LEFT_CENTER,
            APP_NAME,
            demi(18.0),
            TITRE,
        );
        ui.add_space(30.0);
        let items = [
            (Tab::Downloads, Icone::Telecharger, "Downloads", GEL_VERT),
            (Tab::Benchmark, Icone::Comparatif, "Benchmark", GEL_BLEU),
            (
                Tab::Settings,
                Icone::Reglages,
                "Settings",
                gel_de(rgb(0x5f, 0x86, 0xb0)),
            ),
        ];
        let pas = 42.0;
        let (zone, _) = ui.allocate_exact_size(vec2(w, pas * items.len() as f32), Sense::hover());
        let idx = items.iter().position(|i| i.0 == self.tab).unwrap_or(0) as f32;
        let y = ui
            .ctx()
            .animate_value_with_time(egui::Id::unique("nav"), idx, 0.18);
        let ind = Rect::from_min_size(pos2(zone.left(), zone.top() + y * pas), vec2(w, 38.0));
        let p = ui.painter().clone();
        selection(&p, ind, 8.0, 1.0);
        let actifs = self
            .entries
            .iter()
            .filter(|e| e.status == Status::Running)
            .count();
        for (k, (tab, ic, nom, teinte_ic)) in items.into_iter().enumerate() {
            let r = Rect::from_min_size(
                pos2(zone.left(), zone.top() + k as f32 * pas),
                vec2(w, 38.0),
            );
            let rep = ui.interact(r, egui::Id::unique(("nav", k)), Sense::click());
            if rep.clicked() {
                self.tab = tab;
            }
            let h = survol(ui, &rep);
            let actif = self.tab == tab;
            if !actif {
                selection(&p, r, 8.0, 0.45 * h);
            }
            let c = if actif { TEXTE } else { mix(TEXTE2, TEXTE, h) };
            tuile_icone(
                &p,
                Rect::from_min_size(pos2(r.left() + 8.0, r.center().y - 12.0), Vec2::splat(24.0)),
                ic,
                teinte_ic,
            );
            texte(
                ui,
                pos2(r.left() + 42.0, r.center().y),
                Align2::LEFT_CENTER,
                t(l, nom),
                if actif { demi(14.0) } else { moyen(14.0) },
                c,
            );
            if tab == Tab::Downloads && actifs > 0 {
                let b = Rect::from_min_size(
                    pos2(r.right() - 30.0, r.center().y - 10.0),
                    vec2(22.0, 20.0),
                );
                gel(&p, b, 10.0, GEL_BLEU, true, false);
                for (d, c) in [(vec2(0.0, 1.0), nuit(140)), (Vec2::ZERO, Color32::WHITE)] {
                    p.text(
                        b.center() + d,
                        Align2::CENTER_CENTER,
                        actifs.to_string(),
                        demi(12.0),
                        c,
                    );
                }
            }
        }
    }

    /// Titre de page (et texte d'appoint facultatif).
    fn page_title(&self, ui: &mut Ui, titre: &str, sous: Option<&str>) -> Rect {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::hover());
        // Posé sur le ciel : blanc ombré.
        texte_ciel(
            ui.painter(),
            r.left_center(),
            Align2::LEFT_CENTER,
            titre,
            affiche(30.0),
            f32::INFINITY,
        );
        if let Some(s) = sous {
            let rs = texte_ciel(
                ui.painter(),
                ui.cursor().left_top() + vec2(0.0, 2.0),
                Align2::LEFT_TOP,
                s,
                moyen(14.0),
                r.width(),
            );
            ui.allocate_exact_size(vec2(r.width(), rs.height() + 4.0), Sense::hover());
        }
        r
    }

    fn downloads(&mut self, ui: &mut Ui) {
        let l = self.settings.lang;
        let r = self.page_title(ui, t(l, "Downloads"), None);
        // Débit total en direct, à droite du titre.
        let (mut total, mut n) = (0.0, 0);
        for e in self.entries.iter().filter(|e| e.status == Status::Running) {
            n += 1;
            total += self.jobs.get(&e.id).map_or(0.0, |j| j.speed);
        }
        if n > 0 {
            let v = format!("{}/s", octets(total as u64));
            let g = ui.painter().layout_no_wrap(v, demi(14.0), TEXTE);
            let a = format!("{n} {}", compte(l, n, "active"));
            let ga = ui.painter().layout_no_wrap(a, regulier(12.5), TEXTE3);
            let larg = 16.0 + 16.0 + 8.0 + g.size().x + 10.0 + ga.size().x + 16.0;
            let b = Rect::from_min_size(
                pos2(r.right() - larg, r.center().y - 17.0),
                vec2(larg, 34.0),
            );
            let p = ui.painter();
            p.add(verre(ui.ctx(), b, 17, 0.7));
            p.add(reflet(b.shrink(1.0), 16, 0.6));
            orbe(p, pos2(b.left() + 22.0, b.center().y), 9.0, GEL_VERT);
            icone(
                p,
                Rect::from_center_size(pos2(b.left() + 22.0, b.center().y), Vec2::splat(11.0)),
                Icone::Telecharger,
                Color32::WHITE,
            );
            let x = b.left() + 40.0;
            let ga_w = ga.size().x;
            p.galley(pos2(x, b.center().y - g.size().y / 2.0), g.clone(), TEXTE);
            p.galley(
                pos2(b.right() - 16.0 - ga_w, b.center().y - ga.size().y / 2.0),
                ga,
                TEXTE3,
            );
        }
        ui.add_space(18.0);
        self.paste_card(ui);
        ui.add_space(20.0);
        let mut action = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 10.0;
                if self.entries.is_empty() {
                    self.empty_list(ui);
                }
                for (i, e) in self.entries.iter().enumerate() {
                    if let Some(a) = self.entry_card(ui, e) {
                        action = Some((i, a));
                    }
                }
                ui.add_space(14.0);
            });
        if let Some((i, a)) = action {
            self.act(i, a);
        }
    }

    /// Barre « coller un lien » : champ, Coller, Télécharger ; dossier, SHA-256, connexions.
    fn paste_card(&mut self, ui: &mut Ui) {
        let l = self.settings.lang;
        carte(ui, Margin::same(14), |ui| {
            let w = ui.available_width();
            ui.set_width(w);
            let (r, _) = ui.allocate_exact_size(vec2(w, 44.0), Sense::hover());
            let bd = Rect::from_min_size(pos2(r.right() - 136.0, r.top()), vec2(136.0, 44.0));
            // Fenêtre étroite : « Coller » devient un bouton à icône seule.
            let etroit = w < 440.0;
            let lc = if etroit { 44.0 } else { 96.0 };
            let bc = Rect::from_min_size(pos2(bd.left() - 8.0 - lc, r.top()), vec2(lc, 44.0));
            let zone = Rect::from_min_max(r.min, pos2(bc.left() - 8.0, r.bottom()));
            let rep = champ(
                ui,
                zone,
                &mut self.url,
                t(l, "Paste a link…"),
                Some(Icone::Lien),
                regulier(14.5),
            );
            let entree = rep.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let coller = a_la_place(ui, bc, |ui| {
                let n = if etroit { "" } else { t(l, "Paste") };
                bouton_secondaire(ui, n, Some(Icone::Coller), bc.size())
                    .on_hover_text(t(l, "Paste"))
            });
            if coller.clicked() {
                // Lecture directe du presse-papiers (texte seulement).
                if let Some(s) = arboard::Clipboard::new()
                    .and_then(|mut c| c.get_text())
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                {
                    self.url = s;
                }
            }
            let go = a_la_place(ui, bd, |ui| {
                bouton_gel(
                    ui,
                    t(l, "Download"),
                    Some(Icone::Telecharger),
                    bd.size(),
                    GEL_VERT,
                )
            });
            if go.clicked() || entree {
                self.add();
            }
            ui.add_space(10.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, 30.0), Sense::hover());
            // Connexions : − valeur +, à droite.
            let st = Rect::from_min_size(pos2(r.right() - 96.0, r.top()), vec2(96.0, 30.0));
            let p = ui.painter().clone();
            gel(&p, st, 15.0, gel_de(rgb(0xee, 0xf3, 0xf8)), true, false);
            for (k, d) in [(0usize, -1i32), (1, 1)] {
                let b = Rect::from_min_size(
                    pos2(st.left() + k as f32 * 66.0, st.top()),
                    vec2(30.0, 30.0),
                );
                let rep = ui.interact(b, egui::Id::unique(("pas", k)), Sense::click());
                let c = if rep.hovered() { ACCENT } else { TEXTE2 };
                p.line_segment(
                    [b.center() - vec2(4.5, 0.0), b.center() + vec2(4.5, 0.0)],
                    Stroke::new(1.5, c),
                );
                if d > 0 {
                    p.line_segment(
                        [b.center() - vec2(0.0, 4.5), b.center() + vec2(0.0, 4.5)],
                        Stroke::new(1.5, c),
                    );
                }
                if rep.clicked() {
                    self.settings.connections = (self.settings.connections as i32 + d)
                        .clamp(1, engine::MAX_CONNECTIONS as i32)
                        as usize;
                }
            }
            p.text(
                st.center(),
                Align2::CENTER_CENTER,
                self.settings.connections.to_string(),
                demi(13.5),
                TEXTE,
            );
            ui.interact(
                st.shrink2(vec2(30.0, 0.0)),
                egui::Id::unique("pas-aide"),
                Sense::hover(),
            )
            .on_hover_text(t(l, "Connections per download"));
            // Fenêtre étroite : l'étiquette s'efface (l'info-bulle reste).
            let lc = if !etroit {
                texte(
                    ui,
                    pos2(st.left() - 10.0, st.center().y),
                    Align2::RIGHT_CENTER,
                    t(l, "Connections"),
                    regulier(12.5),
                    TEXTE3,
                )
            } else {
                -10.0
            };
            // Ouvrir un .torrent : bouton rond à gauche de la somme de contrôle.
            let bt = Rect::from_min_size(
                pos2(st.left() - 10.0 - lc - 12.0 - 30.0, r.top()),
                vec2(30.0, 30.0),
            );
            if bouton_icone_ici(
                ui,
                bt,
                egui::Id::unique("ouvrir-torrent"),
                Icone::Aimant,
                t(l, "Open a .torrent file"),
            )
            .clicked()
                && self.picker_torrent.is_none()
            {
                self.picker_torrent = Some(choose_torrent(&self.settings.folder));
            }
            let sha =
                Rect::from_min_size(pos2(bt.left() - 8.0 - 112.0, r.top()), vec2(112.0, 30.0));
            champ(ui, sha, &mut self.sha, "SHA-256", None, regulier(12.5))
                .on_hover_text(t(l, "Optional checksum, verified after download."));
            // Dossier : clic = sélecteur natif.
            let d = Rect::from_min_max(r.min, pos2(sha.left() - 12.0, r.bottom()));
            let rep = ui
                .interact(d, egui::Id::unique("dossier"), Sense::click())
                .on_hover_text(format!(
                    "{}\n{}",
                    t(l, "Choose folder"),
                    self.settings.folder
                ));
            let c = if rep.hovered() { ACCENT } else { TEXTE };
            selection(&p, d, 15.0, 0.6 * survol(ui, &rep));
            icone(
                &p,
                Rect::from_min_size(pos2(d.left() + 15.0, d.center().y - 7.0), Vec2::splat(14.0)),
                Icone::Dossier,
                if rep.hovered() { ACCENT } else { TEXTE2 },
            );
            let court = Path::new(&self.settings.folder)
                .file_name()
                .map_or(self.settings.folder.clone(), |n| {
                    n.to_string_lossy().into_owned()
                });
            let g = ligne(ui, court, moyen(12.5), c, d.width() - 42.0);
            p.galley(pos2(d.left() + 40.0, d.center().y - g.size().y / 2.0), g, c);
            if rep.clicked() && self.picker.is_none() {
                self.picker = Some(choose_folder(&self.settings.folder));
            }
        });
    }

    fn empty_list(&self, ui: &mut Ui) {
        let l = self.settings.lang;
        ui.add_space(48.0);
        ui.vertical_centered(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(56.0), Sense::hover());
            let p = ui.painter();
            orbe(p, r.center(), 28.0, GEL_VERT);
            icone_relief(
                p,
                Rect::from_center_size(r.center(), Vec2::splat(24.0)),
                Icone::Telecharger,
            );
            ui.add_space(8.0);
            // Centré, renvoyé à la ligne dans la largeur disponible (moins une marge).
            let w = ui.available_width();
            for (s, police, h) in [
                (t(l, "No downloads yet"), demi(17.0), 24.0),
                (
                    t(
                        l,
                        "Paste a link above. Each file is split across up to 32 parallel connections.",
                    ),
                    moyen(13.5),
                    20.0,
                ),
            ] {
                let r = texte_ciel(
                    ui.painter(),
                    pos2(ui.max_rect().center().x, ui.cursor().top() + 2.0),
                    Align2::CENTER_TOP,
                    s,
                    police,
                    (w - 32.0).max(80.0),
                );
                ui.allocate_exact_size(vec2(w, r.height().max(h - 4.0) + 4.0), Sense::hover());
            }
        });
    }

    fn entry_card(&self, ui: &mut Ui, e: &Entry) -> Option<Act> {
        let l = self.settings.lang;
        let job = self.jobs.get(&e.id);
        let (etat, ce) = self.status_label(e);
        let mut action = None;
        let apparition = self
            .born
            .get(&e.id)
            .map_or(1.0, |b| ((self.now - b) / 0.35).clamp(0.0, 1.0) as f32);
        if apparition < 1.0 {
            ui.ctx().request_repaint();
        }
        let a = 1.0 - (1.0 - apparition).powi(3);
        ui.scope(|ui| {
            ui.multiply_opacity(a);
            let w = ui.available_width();
            let (r, rep) = ui.allocate_exact_size(vec2(w, 84.0), Sense::click());
            let r = r.translate(vec2(0.0, 10.0 * (1.0 - a)));
            let sel = self.selected == Some(e.id);
            let h = survol(ui, &rep);
            let p = ui.painter().clone();
            p.add(verre(ui.ctx(), r, 12, 0.66 + 0.1 * h));
            // Choisie : sélection Windows 7 par-dessus le verre.
            selection(&p, r.shrink(3.0), 9.0, if sel { 1.0 } else { 0.35 * h });
            let nom = e.name.clone().unwrap_or_else(|| e.url.clone());
            let tuile =
                Rect::from_min_size(pos2(r.left() + 16.0, r.top() + 16.0), Vec2::splat(40.0));
            tuile_fichier(&p, tuile, type_de(e, &nom));
            let (x0, x1) = (tuile.right() + 14.0, r.right() - 16.0);
            let actes: &[(Icone, &str, Act)] = match &e.status {
                Status::Failed(m) if m == store::ERR_PARTIEL => &[
                    (Icone::Actualiser, "Start over", Act::Recommencer),
                    (Icone::Fermer, "Remove from list", Act::Remove),
                ],
                Status::Running | Status::Queued => &[
                    (Icone::Pause, "Pause", Act::Pause),
                    (Icone::Dossier, "Open folder", Act::Folder),
                    (Icone::Fermer, "Cancel", Act::Cancel),
                ],
                Status::Failed(m) if m == engine::ERR_PAGE => &[
                    (Icone::Lien, "Open in browser", Act::Navigateur),
                    (Icone::Lecture, "Try as video", Act::Video),
                    (Icone::Fermer, "Remove from list", Act::Remove),
                ],
                Status::Paused | Status::Failed(_) => &[
                    (Icone::Lecture, "Resume", Act::Resume),
                    (Icone::Dossier, "Open folder", Act::Folder),
                    (Icone::Fermer, "Cancel", Act::Cancel),
                ],
                Status::Done | Status::Canceled => &[
                    (Icone::Dossier, "Open folder", Act::Folder),
                    (Icone::Fermer, "Remove from list", Act::Remove),
                ],
            };
            let mut bx = x1 + 6.0;
            for (k, (ic, aide, act)) in actes.iter().enumerate().rev() {
                bx -= 30.0;
                let b = Rect::from_min_size(pos2(bx, r.top() + 11.0), Vec2::splat(30.0));
                if bouton_icone_ici(ui, b, rep.id.with(k), *ic, t(l, aide)).clicked() {
                    action = Some(*act);
                }
                bx -= 2.0;
            }
            let g = ligne(ui, nom, demi(14.5), TEXTE, bx - 10.0 - x0);
            p.galley(pos2(x0, r.top() + 26.0 - g.size().y / 2.0), g, TEXTE);
            let frac = match (e.size, e.status == Status::Done) {
                (_, true) => 1.0,
                (Some(s), _) if s > 0 => e.done as f32 / s as f32,
                _ => 0.0,
            };
            let fa = ui
                .ctx()
                .animate_value_with_time(rep.id.with("progres"), frac, 0.5);
            let barre = Rect::from_min_size(pos2(x0, r.top() + 42.0), vec2(x1 - x0, 9.0));
            match e.status {
                // Barre de progression Vista : vert, reflet qui balaie.
                Status::Running => jauge(&p, barre, fa, rgb(0x3c, 0xbf, 0x2a), true),
                _ => jauge(&p, barre, fa, ce, false),
            }
            let ym = r.top() + 66.0;
            let pr = pastille(ui, pos2(x0, ym), etat, ce);
            let gauche = match (&e.status, e.size) {
                (Status::Failed(m), _) => crate::i18n::tr(l, m).to_string(),
                (_, Some(s)) if e.status == Status::Done => octets(s),
                // En cours, le pourcentage cède la place au débit et au temps restant.
                (Status::Running, Some(s)) => format!("{} / {}", octets(e.done.min(s)), octets(s)),
                (_, Some(s)) => format!(
                    "{} / {}   {:.0}%",
                    octets(e.done.min(s)),
                    octets(s),
                    frac * 100.0
                ),
                (_, None) => octets(e.done),
            };
            let cg = if matches!(e.status, Status::Failed(_)) {
                mix(DANGER, TEXTE2, 0.35)
            } else {
                TEXTE2
            };
            let mut xd = x1;
            if let Some(j) = job.filter(|_| e.status == Status::Running) {
                let lv = j.ctl.lock();
                // Torrent : pairs et envoi en plus.
                if matches!(e.kind, Kind::Torrent { .. }) {
                    let pairs = format!("{} {}", lv.peers, compte(l, lv.peers as usize, "peers"));
                    xd -= texte(
                        ui,
                        pos2(xd, ym),
                        Align2::RIGHT_CENTER,
                        &pairs,
                        regulier(12.5),
                        TEXTE3,
                    ) + 14.0;
                    let up = format!("↑ {}/s", octets(lv.up_speed as u64));
                    xd -= texte(
                        ui,
                        pos2(xd, ym),
                        Align2::RIGHT_CENTER,
                        &up,
                        regulier(12.5),
                        TEXTE3,
                    ) + 14.0;
                }
                drop(lv);
                // Torrent : pas de temps restant (la place va aux pairs et à l'envoi).
                let eta = matches!(e.kind, Kind::Http | Kind::Video { .. });
                if let (Some(s), true, true) = (e.size, j.speed > 1.0, eta) {
                    let eta = format!(
                        "{} {}",
                        duree((s - e.done.min(s)) as f64 / j.speed),
                        t(l, "left")
                    );
                    xd -= texte(
                        ui,
                        pos2(xd, ym),
                        Align2::RIGHT_CENTER,
                        &eta,
                        regulier(12.5),
                        TEXTE3,
                    ) + 14.0;
                }
                xd -= texte(
                    ui,
                    pos2(xd, ym),
                    Align2::RIGHT_CENTER,
                    &format!("{}/s", octets(j.speed as u64)),
                    moyen(12.5),
                    TEXTE,
                ) + 14.0;
            }
            let g = ligne(ui, gauche, regulier(12.5), cg, xd - pr.right() - 10.0);
            p.galley(pos2(pr.right() + 10.0, ym - g.size().y / 2.0), g, cg);
            if rep.clicked() && action.is_none() {
                action = Some(Act::Select);
            }
        });
        action
    }

    fn status_label(&self, e: &Entry) -> (&'static str, Color32) {
        let l = self.settings.lang;
        if e.status == Status::Running && self.partage(e.id) {
            return (t(l, "Seeding"), SUCCES);
        }
        match &e.status {
            Status::Queued => (t(l, "Queued"), TEXTE2),
            Status::Running => (t(l, "Downloading"), ACCENT),
            Status::Paused => (t(l, "Paused"), ATTENTION),
            Status::Done => (t(l, "Completed"), SUCCES),
            Status::Failed(_) => (t(l, "Failed"), DANGER),
            Status::Canceled => (t(l, "Canceled"), TEXTE3),
        }
    }

    fn act(&mut self, i: usize, a: Act) {
        let id = self.entries[i].id;
        match a {
            Act::Select => self.selected = Some(id),
            Act::Folder => open_folder(Path::new(&self.entries[i].dir)),
            Act::Navigateur => open_folder(Path::new(&self.entries[i].url)),
            // Un torrent qui partage s'arrête « fini » (voir torrent::telecharger).
            Act::Pause => match self.jobs.get(&id).filter(|j| j.handle.is_some()) {
                Some(j) => j.ctl.request_stop(),
                None => {
                    self.entries[i].status = Status::Paused;
                    self.last_save = f64::NEG_INFINITY;
                }
            },
            Act::Resume => self.entries[i].status = Status::Queued,
            Act::Recommencer => {
                let e = &mut self.entries[i];
                delete_parts(e, true);
                e.done = 0;
                e.status = Status::Queued;
            }
            Act::Cancel => {
                if self.modale.is_none() {
                    self.modale = Some(Modale::Annuler {
                        id,
                        supprimer: true,
                    });
                }
            }
            Act::Video => {
                let url = self.entries.remove(i).url;
                self.jobs.remove(&id);
                self.ouvrir_video(url);
            }
            Act::Remove => {
                delete_parts(&self.entries[i], false);
                self.entries.remove(i);
                self.born.remove(&id);
                self.jobs.remove(&id);
                if self.selected == Some(id) {
                    self.selected = None;
                }
            }
        }
    }

    /// Annulation confirmée : l'entrée quitte la file (écrite tout de suite) ; son fil s'arrête
    /// en < 1 s et les partiels sont effacés quand il a fini (`orphelins`).
    fn annuler(&mut self, id: u64, effacer: bool) {
        let Some(i) = self.entries.iter().position(|e| e.id == id) else {
            return;
        };
        let e = self.entries.remove(i);
        self.born.remove(&id);
        if self.selected == Some(id) {
            self.selected = None;
        }
        match self.jobs.remove(&id).filter(|j| j.handle.is_some()) {
            Some(j) => {
                j.ctl.request_stop();
                self.orphelins.push((j, e, effacer));
            }
            None => delete_parts(&e, effacer),
        }
        if self.capture.is_none() {
            store::save(QUEUE, &self.entries);
        }
    }

    /// Titre de section : à gauche, et texte discret à droite.
    fn section(&self, ui: &mut Ui, titre: &str, droite: &str) -> Rect {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
        texte(
            ui,
            r.left_center(),
            Align2::LEFT_CENTER,
            titre,
            demi(14.0),
            TITRE,
        );
        texte(
            ui,
            r.right_center(),
            Align2::RIGHT_CENTER,
            droite,
            regulier(12.0),
            TEXTE3,
        );
        r
    }

    /// Panneau de détail : en-tête, chiffres clés, carte du fichier, connexions, débit.
    fn detail(&self, ui: &mut Ui) {
        let l = self.settings.lang;
        let Some(e) = self
            .selected
            .and_then(|id| self.entries.iter().find(|e| e.id == id))
        else {
            ui.add_space((ui.available_height() / 2.0 - 70.0).max(0.0));
            ui.vertical_centered(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(52.0), Sense::hover());
                let p = ui.painter();
                orbe(p, r.center(), 26.0, GEL_BLEU);
                icone_relief(
                    p,
                    Rect::from_center_size(r.center(), Vec2::splat(20.0)),
                    Icone::Comparatif,
                );
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(t(l, "Nothing selected"))
                        .font(demi(15.0))
                        .color(TITRE),
                );
                ui.label(
                    egui::RichText::new(t(l, "Pick a download to watch its connections at work."))
                        .font(regulier(13.0))
                        .color(TEXTE2),
                );
            });
            return;
        };
        let w = ui.available_width();
        let nom = e.name.clone().unwrap_or_else(|| e.url.clone());
        let (r, _) = ui.allocate_exact_size(vec2(w, 46.0), Sense::hover());
        let tuile = Rect::from_min_size(r.min, Vec2::splat(46.0));
        tuile_fichier(ui.painter(), tuile, type_de(e, &nom));
        let x0 = tuile.right() + 14.0;
        let g = ligne(ui, nom, demi(16.5), TEXTE, r.right() - x0);
        ui.painter()
            .galley(pos2(x0, r.top() + 12.0 - g.size().y / 2.0 + 1.0), g, TEXTE);
        let (etat, ce) = self.status_label(e);
        let pr = pastille(ui, pos2(x0, r.top() + 35.0), etat, ce);
        let hote = if matches!(e.kind, Kind::Torrent { .. }) {
            "BitTorrent"
        } else {
            e.url
                .split("://")
                .nth(1)
                .unwrap_or(&e.url)
                .split('/')
                .next()
                .unwrap_or("")
        };
        let g = ligne(
            ui,
            hote,
            regulier(12.5),
            TEXTE3,
            r.right() - pr.right() - 10.0,
        );
        ui.painter().galley(
            pos2(pr.right() + 10.0, r.top() + 35.0 - g.size().y / 2.0),
            g,
            TEXTE3,
        );
        let Some(j) = self.jobs.get(&e.id) else {
            ui.add_space(28.0);
            ui.label(
                egui::RichText::new(if e.status == Status::Done {
                    t(l, "Download complete.")
                } else {
                    t(l, "No live data: resume to see the connections.")
                })
                .font(regulier(13.5))
                .color(TEXTE2),
            );
            return;
        };
        let lv = j.ctl.snapshot();
        let retirees = lv.workers.iter().filter(|w| w.dropped).count();
        if retirees > 0 {
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new(format!(
                    "{} {retirees}/{}.",
                    t(
                        l,
                        "The host refuses extra connections: connections closed automatically:"
                    ),
                    lv.workers.len()
                ))
                .font(regulier(12.5))
                .color(ATTENTION),
            );
        }
        let avis: Vec<&str> = [
            (lv.restarted, "Remote file changed: restarted from zero."),
            (
                !lv.ranges && e.status == Status::Running && e.kind == Kind::Http,
                "Server refuses ranges: 1 connection.",
            ),
        ]
        .into_iter()
        .filter(|&(b, _)| b)
        .map(|(_, m)| t(l, m))
        .collect();
        for m in avis {
            ui.add_space(10.0);
            ui.label(egui::RichText::new(m).font(regulier(12.5)).color(ATTENTION));
        }
        // Fichier creux : l'Explorateur montre la taille finale tout de suite, pas l'espace pris.
        if e.kind == Kind::Http && lv.ranges && e.status != Status::Done {
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new(t(
                    l,
                    "The .part file already shows its final size; disk space is only used as data arrives.",
                ))
                .font(regulier(12.0))
                .color(TEXTE3),
            );
        }
        ui.add_space(18.0);
        // Chiffres clés : grand nombre, unité discrète.
        let (r, _) = ui.allocate_exact_size(vec2(w, 50.0), Sense::hover());
        let en_cours = e.status == Status::Running;
        let taille = lv.size.or(e.size);
        let fait = if e.status == Status::Done {
            taille.unwrap_or(e.done)
        } else {
            lv.done()
        };
        let (vn, vu) = octets_parts(j.speed as u64);
        let pc = taille.map_or(String::from("—"), |s| {
            format!("{:.0}%", fait as f64 * 100.0 / s.max(1) as f64)
        });
        let sur = taille.map_or(String::new(), |s| format!("{} {}", t(l, "of"), octets(s)));
        let eta = match (taille, en_cours && j.speed > 1.0) {
            (Some(s), true) => duree((s - fait.min(s)) as f64 / j.speed),
            _ => "—".into(),
        };
        let stats = [
            (
                t(l, "Speed"),
                if en_cours { vn } else { "—".into() },
                if en_cours {
                    format!("{vu}/s")
                } else {
                    String::new()
                },
            ),
            (t(l, "Progress"), pc, sur),
            (t(l, "Time left"), eta, String::new()),
        ];
        // Colonnes à la mesure de leur contenu : l'avancement porte « sur 5.80 GB ».
        let parts = [0.0, 0.33, 0.76, 1.0];
        for (k, (lab, val, unite)) in stats.iter().enumerate() {
            let x = r.left() + parts[k] * w;
            let col = (parts[k + 1] - parts[k]) * w;
            texte(
                ui,
                pos2(x, r.top() + 7.0),
                Align2::LEFT_CENTER,
                lab,
                regulier(12.0),
                TEXTE3,
            );
            let lw = texte(
                ui,
                pos2(x, r.top() + 33.0),
                Align2::LEFT_CENTER,
                val,
                affiche(26.0),
                TITRE,
            );
            if !unite.is_empty() {
                let g = ligne(ui, unite.as_str(), regulier(12.0), TEXTE2, col - lw - 12.0);
                ui.painter().galley(
                    pos2(x + lw + 5.0, r.top() + 37.0 - g.size().y / 2.0),
                    g,
                    TEXTE2,
                );
            }
        }
        ui.add_space(22.0);
        if matches!(e.kind, Kind::Torrent { .. }) {
            return self.detail_torrent(ui, e, j, &lv);
        }
        if matches!(e.kind, Kind::Video { .. }) {
            self.section(ui, t(l, "Progress"), t(l, "downloaded by yt-dlp"));
            ui.add_space(4.0);
            carte_fichier(ui, &lv, 12.0);
            ui.add_space(20.0);
            return self.detail_courbe(ui, e, j);
        }
        let rs = self.section(ui, t(l, "File map"), t(l, "stolen range"));
        // Légende : le repère ▾ dessiné comme dans la carte.
        let lg = ui
            .painter()
            .layout_no_wrap(t(l, "stolen range").into(), regulier(12.0), TEXTE3)
            .size()
            .x;
        bulle_vol(
            ui.painter(),
            pos2(rs.right() - lg - 10.0, rs.center().y),
            teinte(0.5),
        );
        ui.add_space(4.0);
        carte_fichier(ui, &lv, 12.0);
        ui.add_space(22.0);
        let n = lv.workers.len();
        self.section(
            ui,
            t(l, "Connections"),
            &format!(
                "{n} {}, {} {}",
                if e.status == Status::Running {
                    compte(l, n, "active")
                } else {
                    compte(l, n, "connections")
                },
                lv.steals,
                compte(l, lv.steals as usize, "steals")
            ),
        );
        ui.add_space(6.0);
        let reste = (ui.available_height() - 136.0).max(90.0);
        egui::ScrollArea::vertical()
            .id_salt("connexions")
            .max_height(reste)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                connexions(
                    ui,
                    &lv,
                    &j.speeds,
                    t(l, "stolen"),
                    t(
                        l,
                        "Done early? Take half. This connection finished its own range and took half of the largest remaining one.",
                    ),
                    (t(l, "retries, last error:"), t(l, "Closed: the host refuses this extra connection.")),
                )
            });
        ui.add_space(20.0);
        self.detail_courbe(ui, e, j);
    }

    /// Courbe du débit, en bas du panneau de détail.
    fn detail_courbe(&self, ui: &mut Ui, e: &Entry, j: &Job) {
        let l = self.settings.lang;
        let h: Vec<f32> = j.history.iter().copied().collect();
        let pic = h.iter().copied().fold(0.0, f32::max);
        self.section(
            ui,
            t(l, "Speed"),
            &format!("{} {}/s", t(l, "peak"), octets(pic as u64)),
        );
        ui.add_space(10.0);
        let max =
            ui.ctx()
                .animate_value_with_time(egui::Id::unique(("max", e.id)), pic * 1.12, 0.6);
        courbe(
            ui,
            &h,
            max,
            (ui.available_height() - 4.0).clamp(70.0, 200.0),
        );
    }

    /// Torrent : carte des pièces (à la place des connexions), pairs, envoi, ratio.
    fn detail_torrent(&self, ui: &mut Ui, e: &Entry, j: &Job, lv: &Live) {
        let l = self.settings.lang;
        let n = lv.pieces.len();
        self.section(ui, t(l, "Pieces"), &format!("{n} {}", t(l, "blocks")));
        ui.add_space(4.0);
        carte_pieces(ui, &lv.pieces, 16.0);
        ui.add_space(22.0);
        self.section(
            ui,
            t(l, "Peers"),
            if lv.seeding { t(l, "Seeding") } else { "" },
        );
        ui.add_space(8.0);
        let ratio = lv.uploaded as f64 / lv.size.unwrap_or(0).max(1) as f64;
        let lignes = [
            (t(l, "Connected peers"), lv.peers.to_string()),
            (t(l, "Upload"), format!("{}/s", octets(lv.up_speed as u64))),
            (t(l, "Uploaded"), octets(lv.uploaded)),
            (t(l, "Ratio"), format!("{ratio:.2}")),
        ];
        for (k, v) in lignes {
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
            texte(
                ui,
                r.left_center(),
                Align2::LEFT_CENTER,
                k,
                regulier(13.0),
                TEXTE2,
            );
            texte(
                ui,
                r.right_center(),
                Align2::RIGHT_CENTER,
                &v,
                demi(13.5),
                TEXTE,
            );
        }
        ui.add_space(16.0);
        self.detail_courbe(ui, e, j);
    }

    fn benchmark(&mut self, ui: &mut Ui) {
        let l = self.settings.lang;
        let w = ui.available_width().min(820.0);
        ui.set_max_width(w);
        self.page_title(
            ui,
            t(l, "Benchmark"),
            Some(t(
                l,
                "Downloads the same link with 1 connection, then with N, to a temporary file deleted afterwards.",
            )),
        );
        ui.add_space(20.0);
        let en_cours = self.bench.handle.is_some();
        carte(ui, Margin::same(20), |ui| {
            let w = ui.available_width();
            ui.set_width(w);
            let (r, _) = ui.allocate_exact_size(vec2(w, 44.0), Sense::hover());
            champ(
                ui,
                r,
                &mut self.bench.url,
                t(l, "Paste a link…"),
                Some(Icone::Lien),
                regulier(14.5),
            );
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                ui.set_min_height(40.0);
                ui.label(
                    egui::RichText::new(t(l, "Connections"))
                        .font(moyen(13.5))
                        .color(TEXTE2),
                );
                let mut f = self.bench.connections as f64;
                if curseur(
                    ui,
                    &mut f,
                    (2.0, engine::MAX_CONNECTIONS as f64),
                    false,
                    1.0,
                    170.0,
                )
                .changed()
                {
                    self.bench.connections = f.round() as usize;
                }
                boite_valeur(ui, &self.bench.connections.to_string(), 44.0);
                ui.add_space(20.0);
                ui.label(
                    egui::RichText::new(t(l, "Max seconds per run"))
                        .font(moyen(13.5))
                        .color(TEXTE2),
                );
                ui.add(
                    egui::DragValue::new(&mut self.bench.secs)
                        .range(2..=60)
                        .suffix(" s"),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if !en_cours {
                        let b = bouton_principal(
                            ui,
                            t(l, "Run benchmark"),
                            Some(Icone::Eclair),
                            vec2(176.0, 40.0),
                        );
                        if b.clicked() && self.bench.url.trim().starts_with("http") {
                            self.start_bench();
                        }
                    } else if bouton_secondaire(
                        ui,
                        t(l, "Stop"),
                        Some(Icone::Fermer),
                        vec2(110.0, 40.0),
                    )
                    .clicked()
                    {
                        self.bench.cancel.store(true, Ordering::Relaxed);
                        if let Some((_, c)) =
                            &*self.bench.live.lock().unwrap_or_else(|e| e.into_inner())
                        {
                            c.request_stop();
                        }
                    }
                });
            });
        });
        ui.add_space(16.0);
        let live = self
            .bench
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let (true, Some((phase, c))) = (en_cours, live) {
            carte(ui, Margin::same(20), |ui| {
                ui.set_width(ui.available_width());
                let n = if phase == 1 {
                    1
                } else {
                    self.bench.connections
                };
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(16.0).color(teinte(0.5)));
                    ui.label(
                        egui::RichText::new(format!(
                            "{} {n} {}…",
                            t(l, "Measuring with"),
                            compte(l, n, "connections")
                        ))
                        .font(demi(14.0)),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("{phase} / 2"))
                                .font(regulier(12.5))
                                .color(TEXTE3),
                        );
                    });
                });
                ui.add_space(10.0);
                let lv = c.snapshot();
                carte_fichier(ui, &lv, 10.0);
                ui.add_space(10.0);
                ui.spacing_mut().item_spacing.y = 2.0;
                connexions(
                    ui,
                    &lv,
                    &[],
                    t(l, "stolen"),
                    t(l, "Done early? Take half."),
                    ("", ""),
                );
            });
            ui.add_space(16.0);
        }
        if let Some(r) = &self.bench.result {
            carte(ui, Margin::same(20), |ui| {
                let w = ui.available_width();
                ui.set_width(w);
                match r {
                    Ok((v1, vn)) => {
                        let x = vn / v1.max(1.0);
                        let n = self.bench.connections;
                        let (zr, _) = ui.allocate_exact_size(vec2(w, 72.0), Sense::hover());
                        let gros = texte(
                            ui,
                            zr.left_center() + vec2(-3.0, 0.0),
                            Align2::LEFT_CENTER,
                            &format!("×{x:.1}"),
                            affiche(64.0),
                            if x >= 1.5 { TITRE } else { ATTENTION },
                        );
                        texte(
                            ui,
                            pos2(zr.left() + gros + 16.0, zr.center().y - 11.0),
                            Align2::LEFT_CENTER,
                            t(l, "faster"),
                            demi(19.0),
                            TEXTE,
                        );
                        texte(
                            ui,
                            pos2(zr.left() + gros + 16.0, zr.center().y + 14.0),
                            Align2::LEFT_CENTER,
                            &format!("{} {n} {}", t(l, "with"), compte(l, n, "connections")),
                            regulier(13.5),
                            TEXTE2,
                        );
                        ui.add_space(22.0);
                        let k = ((self.now - self.bench.shown) / 0.9).clamp(0.0, 1.0) as f32;
                        if k < 1.0 {
                            ui.ctx().request_repaint();
                        }
                        let k = 1.0 - (1.0 - k).powi(3);
                        let max = v1.max(*vn).max(1.0);
                        let gris = rgb(0x9a, 0xaa, 0xbd);
                        for (n, v, c) in [(1, *v1, gris), (n, *vn, rgb(0x3c, 0xbf, 0x2a))] {
                            let (r, _) = ui.allocate_exact_size(vec2(w, 20.0), Sense::hover());
                            texte(
                                ui,
                                r.left_center(),
                                Align2::LEFT_CENTER,
                                &format!("{n} {}", compte(l, n, "connections")),
                                moyen(13.5),
                                TEXTE2,
                            );
                            texte(
                                ui,
                                r.right_center(),
                                Align2::RIGHT_CENTER,
                                &format!("{}/s", octets(v as u64)),
                                demi(14.0),
                                TEXTE,
                            );
                            ui.add_space(2.0);
                            let (b, _) = ui.allocate_exact_size(vec2(w, 16.0), Sense::hover());
                            jauge(ui.painter(), b, (v / max) as f32 * k, c, false);
                            ui.add_space(12.0);
                        }
                        if x < 1.5 {
                            ui.label(
                                egui::RichText::new(t(
                                    l,
                                    "Little gain: the server or your line is probably already saturated by one connection.",
                                ))
                                .font(regulier(13.0))
                                .color(TEXTE2),
                            );
                        }
                    }
                    Err(e) => {
                        ui.label(egui::RichText::new(e).font(regulier(13.5)).color(DANGER));
                    }
                }
            });
        }
    }

    fn start_bench(&mut self) {
        let url = self.bench.url.trim().to_string();
        let n = self.bench.connections;
        let max = Duration::from_secs(self.bench.secs as u64);
        let live = self.bench.live.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.bench.cancel = cancel.clone();
        self.bench.result = None;
        self.bench.handle = Some(std::thread::spawn(move || {
            let mut v = [0.0; 2];
            for (k, conns) in [1, n].into_iter().enumerate() {
                let c = Arc::new(Control::default());
                *live.lock().unwrap_or_else(|e| e.into_inner()) = Some((k as u8 + 1, c.clone()));
                v[k] = engine::measure(&url, conns, max, &c)?;
                if cancel.load(Ordering::Relaxed) {
                    return Err("stopped".into());
                }
            }
            Ok((v[0], v[1]))
        }));
    }

    fn settings_tab(&mut self, ui: &mut Ui) {
        let l = self.settings.lang;
        let w = ui.available_width().min(760.0);
        ui.set_max_width(w);
        self.page_title(ui, t(l, "Settings"), None);
        ui.add_space(20.0);
        let mut defile = egui::ScrollArea::vertical().auto_shrink([false, false]);
        if let Some(y) = self.defile {
            defile = defile.vertical_scroll_offset(y);
        }
        defile
            .show(ui, |ui| {
                ui.set_max_width(w);
                carte(ui, Margin::symmetric(20, 6), |ui| {
                    ui.set_width(ui.available_width());
                    titre_section(ui, t(l, "Downloads"));
                    reglage(
                        ui,
                        t(l, "Connections per download"),
                        t(l, "More connections help on fast lines and with slow servers."),
                        |ui| {
                            let n = &mut self.settings.connections;
                            boite_valeur(ui, &n.to_string(), 44.0);
                            let mut f = *n as f64;
                            let r = curseur(ui, &mut f, (1.0, engine::MAX_CONNECTIONS as f64), false, 1.0, 200.0);
                            if r.changed() {
                                *n = f.round() as usize;
                            }
                        },
                    );
                    separateur(ui);
                    reglage(
                        ui,
                        t(l, "Default folder"),
                        t(l, "New downloads are saved here."),
                        |ui| {
                            if bouton_secondaire(ui, t(l, "Browse"), Some(Icone::Dossier), vec2(112.0, 36.0))
                                .clicked()
                                && self.picker.is_none()
                            {
                                self.picker = Some(choose_folder(&self.settings.folder));
                            }
                            let (r, _) = ui.allocate_exact_size(vec2(250.0, 36.0), Sense::hover());
                            champ(ui, r, &mut self.settings.folder, "", None, regulier(13.0));
                        },
                    );
                    separateur(ui);
                    reglage(
                        ui,
                        t(l, "Speed limit"),
                        t(l, "Per download. Off means no limit."),
                        |ui| {
                            let mut on = self.settings.speed_limit_mb > 0.0;
                            if interrupteur(ui, &mut on).changed() {
                                self.settings.speed_limit_mb = if on { 50.0 } else { 0.0 };
                            }
                            if on {
                                ui.add_space(8.0);
                                let v = &mut self.settings.speed_limit_mb;
                                boite_valeur(ui, &format!("{v:.0} MB/s"), 84.0);
                                let mut f = *v as f64;
                                if curseur(ui, &mut f, (1.0, 1000.0), true, 1.0, 160.0).changed() {
                                    *v = f.round() as f32;
                                }
                            }
                        },
                    );
                    separateur(ui);
                    reglage(
                        ui,
                        t(l, "Watch the clipboard"),
                        t(l, "Offers to download a file link as soon as you copy it."),
                        |ui| {
                            if interrupteur(ui, &mut self.settings.clipboard_watch).changed() {
                                self.presse_actif
                                    .store(self.settings.clipboard_watch, Ordering::Relaxed);
                                self.lien_copie = None;
                            }
                        },
                    );
                });
                ui.add_space(20.0);
                self.reglages_v2(ui);
                carte(ui, Margin::symmetric(20, 6), |ui| {
                    ui.set_width(ui.available_width());
                    titre_section(ui, t(l, "Interface"));
                    reglage(
                        ui,
                        t(l, "Language"),
                        t(l, "Applies immediately."),
                        |ui| {
                            segmente(
                                ui,
                                &mut self.settings.lang,
                                &[(Lang::En, "English"), (Lang::Fr, "Français")],
                            );
                        },
                    );
                });
                ui.add_space(20.0);
                carte(ui, Margin::symmetric(20, 6), |ui| {
                    ui.set_width(ui.available_width());
                    titre_section(ui, t(l, "About"));
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
                        ui.painter().image(
                            self.logo.id(),
                            r,
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            ui.add_space(2.0);
                            ui.label(egui::RichText::new(APP_NAME).font(demi(16.0)).color(TITRE));
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} {}, {}",
                                    t(l, "Version"),
                                    env!("CARGO_PKG_VERSION"),
                                    t(l, "MIT license, free and open source.")
                                ))
                                .font(regulier(12.5))
                                .color(TEXTE3),
                            );
                        });
                    });
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new(t(
                            l,
                            "Up to 32 connections per file. Unfinished downloads resume where they stopped, even after a crash.",
                        ))
                        .font(regulier(13.5))
                        .color(TEXTE2),
                    );
                    ui.add_space(14.0);
                });
                ui.add_space(24.0);
            });
    }

    /// Captures d'écran (--capture) : une image par vue, puis fermeture.
    fn capture_step(&mut self, ctx: &egui::Context) {
        let Some((dir, frame)) = &mut self.capture else {
            return;
        };
        *frame += 1;
        let f = *frame;
        let dir = dir.clone();
        // Assez d'images entre le changement de vue et la capture pour finir les animations.
        let vues: [(Tab, Lang, &str); 14] = [
            (Tab::Downloads, Lang::En, "downloads"),
            (Tab::Benchmark, Lang::En, "benchmark"),
            (Tab::Settings, Lang::En, "settings"),
            (Tab::Downloads, Lang::Fr, "downloads_fr"),
            (Tab::Downloads, Lang::Fr, "update_banner_fr"),
            (Tab::Settings, Lang::Fr, "settings_extension_fr"),
            (Tab::Downloads, Lang::Fr, "video_formats_fr"),
            (Tab::Downloads, Lang::Fr, "torrent_pieces_fr"),
            (Tab::Downloads, Lang::Fr, "video_tools_fr"),
            (Tab::Settings, Lang::Fr, "extension_guide_fr"),
            (Tab::Settings, Lang::Fr, "extension_guide_firefox_fr"),
            (Tab::Settings, Lang::En, "extension_guide_en"),
            (Tab::Settings, Lang::Fr, "pairing_fr"),
            (Tab::Downloads, Lang::Fr, "extension_bubble_fr"),
        ];
        for (i, (tab, lang, nom)) in vues.into_iter().enumerate() {
            let k = 30 * (i as u32 + 1);
            if f == k - 24 {
                self.tab = tab;
                self.settings.lang = lang;
                self.demo_vue(nom);
            }
            if f == k {
                ctx.send_viewport_cmd(screenshot(dir.join(format!("{nom}.png"))));
            }
        }
        if f > 30 * vues.len() as u32 + 15 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }

    /// Mise en scène propre à une capture (bulle, fenêtre, sélection, défilement).
    fn demo_vue(&mut self, nom: &str) {
        self.modale = None;
        self.defile = None;
        self.settings.extension_asked = nom != "extension_bubble_fr";
        self.maj.mettre(maj::Etat::Rien);
        self.selected = Some(1);
        let f = |nom: &str, size| crate::github::Fichier {
            name: nom.into(),
            browser_download_url: format!("https://github.com/x/{nom}"),
            size,
        };
        match nom {
            "update_banner_fr" => self.maj.mettre(maj::Etat::Dispo(maj::Annonce {
                version: "0.3.0".into(),
                exe: f("fluxdl.exe", 15_000_000),
                sha256: f("fluxdl.exe.sha256", 80),
            })),
            "settings_extension_fr" => self.defile = Some(300.0),
            "video_formats_fr" => {
                let ch = |q, d, t| (q, d, Some(t));
                self.modale = Some(Modale::Video {
                    url: "https://www.youtube.com/watch?v=aqz-KE-bpKQ".into(),
                    rx: None,
                    infos: Some(Ok(video::Infos {
                        titre: "Big Buck Bunny 60fps 4K - Official Blender Foundation Short Film"
                            .into(),
                        duree: Some(634.0),
                        hauteur_max: 4320,
                        choix: vec![
                            ch(video::Qualite::Best, true, 2_431_000_000),
                            ch(video::Qualite::P1080, true, 402_000_000),
                            ch(video::Qualite::P720, true, 188_000_000),
                            ch(video::Qualite::M4a, true, 10_300_000),
                            ch(video::Qualite::Mp3, true, 10_300_000),
                        ],
                    })),
                    choix: video::Qualite::P1080,
                });
            }
            "torrent_pieces_fr" => self.selected = Some(5),
            "extension_guide_fr" | "extension_guide_en" => self.demo_guide(false),
            "extension_guide_firefox_fr" => self.demo_guide(true),
            "pairing_fr" => {
                self.modale = Some(Modale::Appairage {
                    depuis: std::time::Instant::now(),
                    origine: "chrome-extension://kbfnbcaeplbcioakkpcpgfkobkghlhen".into(),
                    navigateur: "Chrome / Brave",
                })
            }
            "video_tools_fr" => {
                self.modale = Some(Modale::Outils {
                    url: String::new(),
                    etat: EtatOutils::Pret(video::Paquet {
                        ytdlp: Some((f("yt-dlp.exe", 23_627_865), String::new())),
                        ffmpeg: Some((f("ffmpeg.zip", 92_386_792), String::new())),
                    }),
                });
            }
            _ => {}
        }
    }

    /// État de démonstration pour les captures.
    fn demo(&mut self) {
        let mo = 1_000_000u64;
        let taille = 5_800 * mo;
        let mut segs = engine::split(taille, 16, 1 << 20);
        let mut workers = vec![WorkerStat::default(); 16];
        for (i, s) in segs.iter_mut().enumerate() {
            let f = [
                1.0, 0.82, 0.64, 1.0, 0.47, 0.9, 0.71, 1.0, 0.35, 0.58, 0.88, 0.52, 0.76, 1.0,
                0.43, 0.67,
            ][i];
            s.pos = s.start + ((s.end - s.start) as f64 * f) as u64;
            s.worker = Some(i);
            workers[i].segment = Some(i);
        }
        // Connexions finies tôt : chacune a pris la moitié du plus gros reste.
        for (w, de) in [(0usize, 8usize), (3, 14), (7, 4), (13, 11)] {
            let s = segs[de];
            let mid = s.pos + s.remaining() / 2;
            segs[de].end = mid;
            let pos = mid + (s.end - mid) / 3;
            segs.push(Segment {
                start: mid,
                pos,
                end: s.end,
                worker: Some(w),
                stolen: true,
            });
            workers[w].segment = Some(segs.len() - 1);
        }
        let live = Live {
            file_name: Some("ubuntu-24.04-desktop-amd64.iso".into()),
            size: Some(taille),
            ranges: true,
            segments: segs,
            workers,
            steals: 4,
            ..Default::default()
        };
        let speeds = (0..16)
            .map(|i| 5.2e6 + (i as f64 * 1.7e6) % 3.1e6)
            .collect::<Vec<_>>();
        let mut job = Job::new(Arc::new(Control::with_live(live.clone())), None);
        job.speed = speeds.iter().sum();
        job.speeds = speeds;
        job.history = (0..120)
            .map(|i| {
                let x = i as f32;
                job_speed_curve(x) * 1e6
            })
            .collect();
        let entree = |id, url: &str, name: Option<&str>, status, size: Option<u64>, done| Entry {
            id,
            url: url.into(),
            dir: self.settings.folder.clone(),
            name: name.map(str::to_string),
            sha256: None,
            connections: 16,
            status,
            size,
            done,
            kind: Kind::Http,
            referrer: None,
            cookies: None,
        };
        self.entries = vec![
            entree(
                1,
                "https://releases.ubuntu.com/24.04/ubuntu-24.04-desktop-amd64.iso",
                live.file_name.as_deref(),
                Status::Running,
                Some(taille),
                live.done(),
            ),
            entree(
                2,
                "https://download.blender.org/release/Blender4.2/blender-4.2.3-windows-x64.zip",
                Some("blender-4.2.3-windows-x64.zip"),
                Status::Done,
                Some(381 * mo),
                381 * mo,
            ),
            entree(
                3,
                "https://huggingface.co/model.safetensors",
                Some("model-00001-of-00004.safetensors"),
                Status::Paused,
                Some(4_970 * mo),
                1_208 * mo,
            ),
            entree(
                4,
                "https://example.org/archive.tar.zst",
                Some("archive.tar.zst"),
                Status::Failed("HTTP 404".into()),
                None,
                0,
            ),
        ];
        self.jobs.insert(1, job);
        // Torrent en cours : carte des pièces (reçues par paquets, comme en vrai).
        let tt = 3_950 * mo;
        let pieces: Vec<u8> = (0..torrent::CASES)
            .map(|i| {
                let x = i as f32;
                let v = (x * 0.37).sin() + (x * 0.11).cos() + if i < 70 { 1.2 } else { -0.4 };
                if v > 0.6 {
                    255
                } else if v > 0.1 {
                    140
                } else {
                    0
                }
            })
            .collect();
        let recu = (tt as f64 * pieces.iter().map(|&v| v as f64).sum::<f64>()
            / (255.0 * pieces.len() as f64)) as u64;
        let lt = Live {
            file_name: Some("debian-13.2.0-amd64-DVD-1.iso".into()),
            size: Some(tt),
            segments: vec![Segment {
                start: 0,
                pos: recu,
                end: tt,
                worker: Some(0),
                stolen: false,
            }],
            workers: vec![WorkerStat::default()],
            peers: 37,
            uploaded: 412 * mo,
            up_speed: 1.8e6,
            pieces,
            ..Default::default()
        };
        let mut jt = Job::new(Arc::new(Control::with_live(lt)), None);
        jt.speed = 24.6e6;
        jt.history = (0..120)
            .map(|i| job_speed_curve(i as f32) * 0.26e6)
            .collect();
        let mut tor = entree(
            5,
            "magnet:?xt=urn:btih:0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c",
            Some("debian-13.2.0-amd64-DVD-1.iso"),
            Status::Running,
            Some(tt),
            recu,
        );
        tor.kind = Kind::Torrent {
            hash: "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c".into(),
            files: None,
            started: true,
        };
        let mut vid = entree(
            6,
            "https://www.youtube.com/watch?v=aqz-KE-bpKQ",
            Some("Big Buck Bunny 60fps 4K"),
            Status::Done,
            Some(402 * mo),
            402 * mo,
        );
        vid.kind = Kind::Video {
            quality: video::Qualite::P1080,
            estimate: Some(402 * mo),
        };
        self.entries.insert(1, tor);
        self.entries.insert(3, vid);
        self.jobs.insert(5, jt);
        self.selected = Some(1);
        self.bench.url = "https://proof.ovh.net/files/100Mb.dat".into();
        self.bench.result = Some(Ok((11.4e6, 86.9e6)));
        self.bench.shown = -10.0;
        self.settings.speed_limit_mb = 50.0;
        self.settings.token = "3fa2c9d8e1b04f5a8c7d6e5f4a3b2c1d".into();
        self.next_id = 7;
    }
}

/// Courbe de débit plausible pour la démo (Mo/s).
fn job_speed_curve(x: f32) -> f32 {
    let monte = 1.0 - (-x / 12.0).exp();
    96.0 * monte + 6.0 * (x * 0.37).sin() + 5.0 * (x * 0.11).cos()
}

#[derive(Clone, Copy)]
enum Act {
    Select,
    /// Lien de page web : réessayer avec yt-dlp.
    Video,
    /// Lien de page web : l'ouvrir dans le navigateur.
    Navigateur,
    Folder,
    Pause,
    Resume,
    /// Fichier partiel introuvable : repartir de zéro.
    Recommencer,
    Cancel,
    Remove,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        if self.capture.is_none() {
            self.schedule(now);
        }
        self.now = now;
        if self.bench.handle.as_ref().is_some_and(|h| h.is_finished()) {
            let h = self.bench.handle.take().map(|h| h.join());
            self.bench.result = h.map(|r| r.unwrap_or_else(|_| Err("panic".into())));
            self.bench.shown = now;
        }
        if let Some(rx) = &self.picker {
            match rx.try_recv() {
                Ok(d) => {
                    self.settings.folder = d;
                    self.picker = None;
                }
                Err(TryRecvError::Disconnected) => self.picker = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some(rx) = &self.picker_torrent {
            match rx.try_recv() {
                Ok(f) => {
                    self.picker_torrent = None;
                    self.ajouter(f, None, None, true);
                }
                Err(TryRecvError::Disconnected) => self.picker_torrent = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        // Liens de l'extension ou d'une 2e instance (liens magnet, .torrent ouverts).
        let recus: Vec<Demande> = self
            .demandes
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for d in recus {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.ajouter(d.url, d.referrer, d.cookies, d.locale);
        }
        // .torrent glissés sur la fenêtre.
        let poses: Vec<String> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().display().to_string())
                .filter(|p| !p.is_empty())
                .collect()
        });
        for f in poses {
            if f.to_ascii_lowercase().ends_with(".url") {
                // Raccourci Internet (Windows) : la ligne « URL=… ».
                let lien = std::fs::read_to_string(&f).ok().and_then(|t| {
                    t.lines()
                        .find_map(|l| l.trim().strip_prefix("URL=").map(str::to_string))
                });
                if let Some(u) = lien {
                    self.ajouter(u, None, None, false);
                }
            } else {
                self.ajouter(f, None, None, true);
            }
        }
        // Lien copié ailleurs : bulle, sauf copie de l'appli ou lien déjà dans la file.
        if let Some(l) = self.presse.as_ref().and_then(|rx| rx.try_iter().last()) {
            if self.settings.clipboard_watch
                && self.copie_app.as_ref() != Some(&l)
                && !self.entries.iter().any(|e| e.url == l)
            {
                self.lien_copie = Some((l, now));
            }
        }
        if self.capture.is_none() {
            if now >= self.maj_prochaine {
                self.maj.chercher(true);
                self.maj_prochaine = now + 6.0 * 3600.0;
                ctx.request_repaint_after(Duration::from_secs(6 * 3600));
            }
            // Accord donné : nouvelle version prête → installée, fermeture propre, relance.
            if self.maj_auto && matches!(self.maj.etat(), maj::Etat::Pret(..)) {
                self.maj_auto = false;
                if let Some(exe) = self.maj.installer() {
                    self.relance = Some(exe);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        if self.capture.is_none() {
            self.suivre_appairage(&ctx);
        }
        self.suivre_modale();
        self.capture_step(&ctx);
        let ecran = ctx.content_rect();
        fond(&ctx.layer_painter(egui::LayerId::background()), ecran);
        // Panneaux de verre posés sur le fond, à 12 px des bords de la fenêtre.
        let panneau = |ui: &mut Ui, marge: Margin, voile: f32| {
            let r = ui.max_rect();
            let r = Rect::from_min_max(
                r.min - vec2(marge.left as f32 - 12.0, marge.top as f32 - 12.0),
                r.max + vec2(marge.right as f32 - 12.0, marge.bottom as f32 - 12.0),
            );
            egui::Painter::new(ui.ctx().clone(), ui.layer_id(), ecran).add(verre(
                ui.ctx(),
                r,
                16,
                voile,
            ));
        };
        let marge = Margin {
            left: 26,
            right: 26,
            top: 30,
            bottom: 26,
        };
        egui::Panel::left("navigation")
            .exact_size(256.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::NONE.inner_margin(marge))
            .show(ui, |ui| {
                panneau(ui, marge, 0.5);
                self.sidebar(ui)
            });
        if self.tab == Tab::Downloads {
            let marge = Margin {
                left: 28,
                right: 38,
                top: 36,
                bottom: 30,
            };
            egui::Panel::right("detail")
                .exact_size((ui.available_width() * 0.43).clamp(400.0, 600.0))
                .resizable(false)
                .show_separator_line(false)
                .frame(egui::Frame::NONE.inner_margin(marge))
                .show(ui, |ui| {
                    panneau(ui, marge, 0.66);
                    self.detail(ui)
                });
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.inner_margin(Margin {
                left: 18,
                right: 18,
                top: 22,
                bottom: 0,
            }))
            .show(ui, |ui| match self.tab {
                Tab::Downloads => self.downloads(ui),
                Tab::Benchmark => self.benchmark(ui),
                Tab::Settings => self.settings_tab(ui),
            });
        self.modale(&ctx);
        self.bulle_maj(&ctx);
        self.bulle_extension(&ctx);
        self.bulle_lien(&ctx);
        self.zone_depot(&ctx);
        self.persist(now);
        // Textes copiés par l'appli (Ctrl+C, bouton Copier) : à ne pas proposer.
        if let Some(c) = ctx.output(|o| {
            o.commands.iter().rev().find_map(|c| match c {
                egui::OutputCommand::CopyText(t) => Some(t.trim().to_string()),
                _ => None,
            })
        }) {
            self.copie_app = Some(c);
        }
        // Au repos, aucune image : on ne redessine que si quelque chose bouge ou télécharge
        // (les animations egui demandent elles-mêmes leurs images).
        let vivant = self
            .entries
            .iter()
            .any(|e| matches!(e.status, Status::Running | Status::Queued))
            || self.bench.handle.is_some()
            || self.picker.is_some()
            || self.picker_torrent.is_some()
            || matches!(
                self.maj.etat(),
                maj::Etat::Recherche | maj::Etat::Telechargement(..)
            )
            || self.modale.is_some();
        if vivant {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.capture.is_some() {
            return;
        }
        // Arrêt propre : l'état de chaque téléchargement est sauvé, il reprendra au lancement.
        for j in self
            .jobs
            .values()
            .chain(self.orphelins.iter().map(|o| &o.0))
        {
            j.ctl.request_stop();
        }
        let fin = std::time::Instant::now() + Duration::from_secs(2);
        while self
            .jobs
            .values()
            .chain(self.orphelins.iter().map(|o| &o.0))
            .any(|j| j.handle.as_ref().is_some_and(|h| !h.is_finished()))
            && std::time::Instant::now() < fin
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        for (_, e, effacer) in &self.orphelins {
            delete_parts(e, *effacer);
        }
        store::save(QUEUE, &self.entries);
        store::save(SETTINGS, &self.settings);
        if let Some(exe) = &self.relance {
            maj::relancer(exe);
        }
    }
}

/// Annulé (`annule`) : efface le `.part` et l'état d'un fichier HTTP, ou les fichiers
/// partiels d'un torrent. Torrent retiré de la liste : sort de la session, fichiers gardés.
/// raccourci : les `.part` de yt-dlp restent (noms choisis par yt-dlp) ; à faire si demandé.
fn delete_parts(e: &Entry, annule: bool) {
    match &e.kind {
        Kind::Http if annule => {
            if let Some(n) = &e.name {
                let cible = Path::new(&e.dir).join(n);
                let _ = std::fs::remove_file(engine::part_path(&cible));
                let _ = std::fs::remove_file(engine::state_path(&cible));
            }
        }
        Kind::Torrent { hash, .. } => {
            if let Some(m) = torrent::lance() {
                m.oublier(hash, annule && e.status != Status::Done);
            }
            let _ = std::fs::remove_file(torrent::garde(hash));
        }
        _ => {}
    }
}

/// Tuile d'une entrée : vidéo et torrent d'après leur sorte, sinon d'après l'extension.
fn type_de(e: &Entry, nom: &str) -> TypeFichier {
    match e.kind {
        Kind::Video { .. } => TypeFichier::Video,
        Kind::Torrent { .. } => TypeFichier::Torrent,
        Kind::Http => TypeFichier::de(nom),
    }
}

/// Secondes Unix.
fn secondes() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn open_folder(d: &Path) {
    let prog = if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(prog).arg(d).spawn();
}

pub fn octets(n: u64) -> String {
    let (v, u) = octets_parts(n);
    format!("{v} {u}")
}

/// Nombre et unité séparés (le nombre en grand, l'unité en discret).
fn octets_parts(n: u64) -> (String, &'static str) {
    let n = n as f64;
    match n {
        n if n >= 1e9 => (format!("{:.2}", n / 1e9), "GB"),
        n if n >= 1e6 => (format!("{:.1}", n / 1e6), "MB"),
        n if n >= 1e3 => (format!("{:.0}", n / 1e3), "kB"),
        n => (format!("{n:.0}"), "B"),
    }
}

/// Ligne de réglage : titre et aide à gauche (centrés verticalement), contrôle à droite.
fn reglage(ui: &mut Ui, titre: &str, aide: &str, controle: impl FnOnce(&mut Ui)) {
    let w = ui.available_width();
    let lw = w * 0.5;
    let (r, _) = ui.allocate_exact_size(vec2(w, 64.0), Sense::hover());
    let p = ui.painter();
    let gt = p.layout_no_wrap(titre.to_string(), moyen(14.0), TEXTE);
    let ga = p.layout(aide.to_string(), regulier(12.5), TEXTE3, lw);
    let y = r.center().y - (gt.size().y + 2.0 + ga.size().y) / 2.0;
    let ht = gt.size().y;
    p.galley(pos2(r.left(), y), gt, TEXTE);
    p.galley(pos2(r.left(), y + ht + 2.0), ga, TEXTE3);
    let cr = Rect::from_min_max(pos2(r.left() + lw + 16.0, r.top()), r.max);
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(cr)
            .layout(Layout::right_to_left(Align::Center)),
        controle,
    );
}

/// Titre au-dessus d'une carte de réglages.
/// Titre en tête d'une carte de réglages (bleu des panneaux Windows 7).
fn titre_section(ui: &mut Ui, t: &str) {
    ui.add_space(10.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    texte(
        ui,
        r.left_center(),
        Align2::LEFT_CENTER,
        t,
        demi(15.0),
        TITRE,
    );
}

/// Nom compté : « 1 vol » / « 4 vols » (français : 0 et 1 au singulier).
fn compte(l: Lang, n: usize, pluriel: &'static str) -> &'static str {
    let m = t(l, pluriel);
    let singulier = match l {
        Lang::En => n == 1,
        Lang::Fr => n <= 1,
    };
    if singulier {
        m.strip_suffix('s').unwrap_or(m)
    } else {
        m
    }
}

fn separateur(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
    ui.painter().hline(
        r.x_range(),
        r.center().y.round() + 0.5,
        Stroke::new(1.0, BORD),
    );
}

/// Pose un bouton dessiné à la main à un emplacement précis.
fn a_la_place(ui: &mut Ui, r: Rect, f: impl FnOnce(&mut Ui) -> Response) -> Response {
    ui.scope_builder(UiBuilder::new().max_rect(r), f).inner
}

/// Sélecteur de dossier natif dans un fil (l'interface reste vivante) ; rien si annulé.
fn choose_folder(depart: &str) -> Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let depart = depart.to_string();
    std::thread::spawn(move || {
        if let Some(d) = pick_folder(&depart) {
            let _ = tx.send(d);
        }
    });
    rx
}

/// Sélecteur de fichier .torrent dans un fil.
fn choose_torrent(depart: &str) -> Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let depart = depart.to_string();
    std::thread::spawn(move || {
        if let Some(f) = pick_torrent(&depart) {
            let _ = tx.send(f);
        }
    });
    rx
}

#[cfg(windows)]
fn pick_torrent(depart: &str) -> Option<String> {
    rfd::FileDialog::new()
        .set_directory(depart)
        .add_filter("Torrent", &["torrent"])
        .pick_file()
        .map(|p| p.display().to_string())
}

#[cfg(not(windows))]
fn pick_torrent(depart: &str) -> Option<String> {
    let essais: [(&str, Vec<String>); 2] = [
        (
            "zenity",
            vec![
                "--file-selection".into(),
                "--file-filter=*.torrent".into(),
                format!("--filename={depart}/"),
            ],
        ),
        (
            "kdialog",
            vec![
                "--getopenfilename".into(),
                depart.into(),
                "*.torrent".into(),
            ],
        ),
    ];
    for (prog, args) in essais {
        if let Ok(o) = std::process::Command::new(prog).args(&args).output() {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            return (o.status.success() && !s.is_empty()).then_some(s);
        }
    }
    None
}

#[cfg(windows)]
fn pick_folder(depart: &str) -> Option<String> {
    rfd::FileDialog::new()
        .set_directory(depart)
        .pick_folder()
        .map(|p| p.display().to_string())
}

/// raccourci : zenity (GNOME) puis kdialog (KDE) ; sans eux, le champ texte des réglages
/// reste le seul moyen. Le portail XDG demanderait zbus (lourd) : à voir si on publie Linux.
#[cfg(not(windows))]
fn pick_folder(depart: &str) -> Option<String> {
    let essais: [(&str, Vec<String>); 2] = [
        (
            "zenity",
            vec![
                "--file-selection".into(),
                "--directory".into(),
                format!("--filename={depart}/"),
            ],
        ),
        (
            "kdialog",
            vec!["--getexistingdirectory".into(), depart.into()],
        ),
    ];
    for (prog, args) in essais {
        if let Ok(o) = std::process::Command::new(prog).args(&args).output() {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            return (o.status.success() && !s.is_empty()).then_some(s);
        }
    }
    None
}

fn duree(s: f64) -> String {
    let s = s.min(359_999.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

fn screenshot(f: PathBuf) -> egui::ViewportCommand {
    egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let rgba: Vec<u8> = img
            .pixels
            .iter()
            .flat_map(|c| c.to_srgba_unmultiplied())
            .collect();
        let r = image::save_buffer(
            &f,
            &rgba,
            img.width() as u32,
            img.height() as u32,
            image::ColorType::Rgba8,
        );
        println!("capture {} : {:?}", f.display(), r.err());
    }))
}
