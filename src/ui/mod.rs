//! Fenêtre egui : barre latérale, file de téléchargements, panneau de détail par connexion,
//! comparatif, réglages.

mod draw;
mod fenetres;
mod guide;

use crate::apres::{self, Categorie};
use crate::i18n::t;
use crate::maj::{self, Maj};
use crate::serveur::{self, Demande, Statut};
use crate::store::{self, Entry, Fin, Kind, Lang, Settings, Status};
use crate::{plan, torrent, video};
use dlcore::engine::{self, Control, Live, Options, Outcome, Seau, Segment, WorkerStat};
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
/// Reste à télécharger sous lequel un fichier démarre même si la file est pleine.
const PETIT: u64 = 10_000_000;
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

/// Outil d'un fichier fini, lancé depuis le panneau de détail.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Outil {
    Extraire,
    Sha,
}

/// Extraction ou empreinte en cours dans un fil : avancement, arrêt, résultat (dossier extrait
/// ou empreinte hex).
struct TacheFichier {
    suivi: Arc<apres::Suivi>,
    rx: Receiver<Result<String, String>>,
}

/// Résultat attendu d'un fil (infos vidéo, aperçu de torrent…).
type Attente<T> = Option<Receiver<T>>;

/// yt-dlp / ffmpeg à télécharger (Windows) : lecture des releases, accord, installation.
enum EtatOutils {
    Lecture(Receiver<Result<video::Paquet, &'static str>>),
    Pret(Box<video::Paquet>),
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
        /// Passage « de … à … » (texte saisi, vide = toute la vidéo) ; live depuis le début.
        de: String,
        a: String,
        debut: bool,
        /// Sous-titres, miniature, chapitres, conteneur (défaut : réglages).
        sortie: video::Sortie,
    },
    /// Vidéo ouverte dans une playlist : cette vidéo seule ou toute la playlist ?
    VideoOuListe(String),
    /// Vidéos d'une playlist / chaîne (lues par yt-dlp dans un fil, `LISTE_MAX` à la fois) :
    /// cases, une qualité et les mêmes ajouts pour toutes.
    Liste {
        url: String,
        rx: Attente<Result<video::Playlist, String>>,
        /// Arrête la lecture en cours (fenêtre fermée).
        stop: Arc<AtomicBool>,
        liste: Option<Result<video::Playlist, String>>,
        /// Échec de « Charger plus » (la liste déjà lue reste).
        erreur_suite: Option<String>,
        coches: Vec<bool>,
        filtre: String,
        choix: video::Qualite,
        sortie: video::Sortie,
        /// Sous-dossier au nom de la playlist.
        dossier: bool,
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
    /// Lien expiré : nouveau lien pour le même fichier.
    NouveauLien { id: u64, lien: String },
    /// Installation guidée de l'extension.
    Guide(guide::Guide),
    /// Une extension demande l'appairage : « Autoriser ? ».
    Appairage {
        depuis: std::time::Instant,
        origine: String,
        navigateur: &'static str,
    },
    /// Tout est fini : veille ou arrêt du PC dans 60 s (depuis `depuis`, horloge de l'appli),
    /// sauf « Annuler ».
    FinCompte { eteindre: bool, depuis: f64 },
}

/// Compte à rebours avant la veille ou l'arrêt du PC.
const COMPTE_FIN_S: f64 = 60.0;
/// Limites proposées dans le menu (Mo/s, 0 = illimitée).
const LIMITES: [f32; 5] = [0.0, 1.0, 5.0, 10.0, 50.0];

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
    /// Horloge de `now` (aussi quand la fenêtre est masquée).
    horloge: std::time::Instant,
    /// Entrées de queue.json illisibles par cette version : gardées telles quelles.
    inconnues: Vec<serde_json::Value>,
    /// Police CJK du système déjà cherchée.
    cjk: bool,
    /// Cookies de connexion par page vidéo (extension, option par site) : en mémoire seulement,
    /// oubliés quand plus aucune vidéo de la page n'attend (fin, retrait, fenêtre fermée).
    jarres: HashMap<String, Arc<Vec<crate::cookies::Cookie>>>,
    /// Limite globale, partagée par les connexions de tous les téléchargements HTTP et vidéo.
    globale: Arc<Seau>,
    /// Limite d'un téléchargement choisie dans le panneau de détail (dessiné sans `&mut`).
    limite_choisie: std::cell::Cell<Option<(u64, f32)>>,
    /// Planification : file en attente de l'heure de démarrage ; dernière minute vue ; mis en
    /// pause par l'heure d'arrêt (repartent en attente, pas « en pause par l'utilisateur »).
    plan_arme: bool,
    plan_vu: u16,
    plan_suspendus: Vec<u64>,
    /// Un téléchargement a fini pendant cette session (l'action de fin n'a lieu qu'après).
    fini_session: bool,
    /// Veille (`false`) ou arrêt du PC (`true`) : `systeme::veille_ou_arret`, faux en test.
    executer: fn(bool),
    /// Extractions et empreintes en cours, puis leur résultat, par (téléchargement, outil).
    taches: HashMap<(u64, Outil), TacheFichier>,
    resultats: HashMap<(u64, Outil), Result<String, String>>,
    /// Outil lancé ou arrêté dans le panneau de détail (dessiné sans `&mut`).
    outil_demande: std::cell::Cell<Option<(u64, Outil)>>,
    /// Empreinte collée pour la comparer à celle calculée.
    sha_colle: std::cell::RefCell<String>,
    /// Le sélecteur de dossier ouvert choisit le dossier de cette catégorie (tri par type).
    picker_cat: Option<Categorie>,
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
            horloge: std::time::Instant::now(),
            inconnues: Vec::new(),
            cjk: false,
            jarres: HashMap::new(),
            globale: Arc::new(Seau::new(0)),
            limite_choisie: Default::default(),
            plan_arme: false,
            plan_vu: crate::systeme::minute_locale(),
            plan_suspendus: Vec::new(),
            fini_session: false,
            executer: crate::systeme::veille_ou_arret,
            taches: HashMap::new(),
            resultats: HashMap::new(),
            outil_demande: Default::default(),
            sha_colle: Default::default(),
            picker_cat: None,
        };
        if let Some(d) = capture {
            // Captures : jamais de vraie veille ni d'arrêt du PC.
            app.executer = |_| {};
            app.demo();
            app.capture = Some((d, 0));
            return app;
        }
        if let Some(s) = store::load(SETTINGS) {
            app.settings = s;
        }
        // Veille et arrêt du PC : jamais repris d'une session à l'autre.
        if matches!(app.settings.end_action, Fin::Veille | Fin::Eteindre) {
            app.settings.end_action = Fin::Rien;
        }
        app.plan_arme =
            plan::arme_au_depart(app.plan_vu, app.settings.plan_start, app.settings.plan_stop);
        (app.entries, app.inconnues) = store::load_queue(&store::data_dir(), QUEUE);
        app.presse_actif
            .store(app.settings.clipboard_watch, Ordering::Relaxed);
        app.presse = Some(crate::presse::lancer(ctx.clone(), app.presse_actif.clone()));
        for e in &mut app.entries {
            store::au_lancement(e);
        }
        app.next_id = app.entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        maj::nettoyer();
        // Cookies laissés par un plantage (instance unique déjà prise).
        crate::cookies::purger(&crate::cookies::dossier());
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
            self.settings.token.clone(),
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
    /// Le sort est renvoyé à l'extension (elle n'abandonne sa copie que si ajouté / actif).
    fn ajouter(
        &mut self,
        url: String,
        referrer: Option<String>,
        cookies: Option<String>,
        local: bool,
    ) -> Statut {
        let url = url.trim().to_string();
        if url.starts_with("magnet:") {
            self.ouvrir_torrent(url, None);
            return Statut::Ajoute;
        }
        if local && url.to_ascii_lowercase().ends_with(".torrent") {
            let nom = Path::new(&url)
                .file_name()
                .map_or(url.clone(), |n| n.to_string_lossy().into_owned());
            let Ok(o) = std::fs::read(&url) else {
                return Statut::Refuse;
            };
            self.ouvrir_torrent(nom, Some(o));
            return Statut::Ajoute;
        }
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Statut::Refuse;
        }
        if video::site_video(&url) {
            self.ouvrir_video(url);
            return Statut::Ajoute;
        }
        // Déjà dans la file : la montrer plutôt que télécharger deux fois le même fichier ;
        // en pause → reprise ; en échec → refus (le navigateur garde sa copie) ; fini → rien
        // tant que le fichier est là.
        if let Some(e) = self.entries.iter_mut().rev().find(|e| {
            e.url == url
                && match &e.status {
                    Status::Done => e
                        .name
                        .as_ref()
                        .is_some_and(|n| Path::new(&e.dir).join(n).exists()),
                    s => *s != Status::Canceled,
                }
        }) {
            let st = match e.status {
                Status::Done => Statut::DejaFini,
                Status::Failed(_) => Statut::Refuse,
                Status::Paused => {
                    e.status = Status::Queued;
                    Statut::DejaActif
                }
                _ => Statut::DejaActif,
            };
            self.selected = Some(e.id);
            self.tab = Tab::Downloads;
            return st;
        }
        let sha = std::mem::take(&mut self.sha).trim().to_string();
        let e = self.nouvelle(url, Kind::Http);
        e.sha256 = (!sha.is_empty()).then_some(sha);
        e.referrer = referrer;
        e.cookies = cookies;
        Statut::Ajoute
    }

    /// Ajoute une entrée en attente, choisie dans la liste.
    fn nouvelle(&mut self, url: String, kind: Kind) -> &mut Entry {
        // Tri par type : vidéo → Vidéos (son seul → Musique) ; fichier HTTP : au nom donné par
        // la sonde (`start`) ; torrent : jamais.
        let dir = match &kind {
            Kind::Video { quality, .. } => self.dossier_tri(video_ou_son(*quality)),
            _ => self.settings.folder.clone(),
        };
        self.entries.push(Entry {
            id: self.next_id,
            url,
            dir,
            name: None,
            sha256: None,
            connections: self.settings.connections,
            status: Status::Queued,
            size: None,
            done: 0,
            kind,
            referrer: None,
            cookies: None,
            speed_limit_mb: 0.0,
        });
        self.selected = Some(self.next_id);
        self.born.insert(self.next_id, self.now);
        self.next_id += 1;
        self.tab = Tab::Downloads;
        self.entries.last_mut().expect("entrée ajoutée")
    }

    /// Dossier d'un nouveau téléchargement de cette catégorie : celui de la catégorie si le tri
    /// par type est actif, sinon le dossier par défaut.
    fn dossier_tri(&self, c: Categorie) -> String {
        if !self.settings.sort_by_type {
            return self.settings.folder.clone();
        }
        self.dossier_categorie(c)
    }

    /// Dossier choisi pour la catégorie, sinon `<dossier par défaut>/<catégorie traduite>`.
    fn dossier_categorie(&self, c: Categorie) -> String {
        match self.settings.sort_folders.get(&c).map(|d| d.trim()) {
            Some(d) if !d.is_empty() => d.to_string(),
            _ => Path::new(&self.settings.folder)
                .join(t(self.settings.lang, c.nom()))
                .to_string_lossy()
                .into_owned(),
        }
    }

    /// Vidéo : formats lus par yt-dlp ; outils manquants → accord (Windows) ou message.
    /// Playlist / chaîne → liste des vidéos ; vidéo dans une playlist → question.
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
        match video::liste(&url) {
            Some((video::GenreListe::Liste, _)) => self.ouvrir_liste(url),
            Some((video::GenreListe::VideoDeListe, _)) => {
                self.modale = Some(Modale::VideoOuListe(url))
            }
            None => self.ouvrir_video_seule(url),
        }
    }

    /// Formats d'une seule vidéo lus par yt-dlp dans un fil (outils déjà vérifiés).
    fn ouvrir_video_seule(&mut self, url: String) {
        let (tx, rx) = std::sync::mpsc::channel();
        let o = self.outils.clone();
        let u = url.clone();
        let jarre = self.jarres.get(&url).cloned();
        std::thread::spawn(move || {
            let r = if o.ytdlp.is_none() {
                Err(video::ABSENT.to_string())
            } else {
                // Fichier de cookies effacé dès que yt-dlp a lu la page.
                match jarre.map(|j| crate::cookies::Fichier::creer(&crate::cookies::dossier(), &j))
                {
                    Some(Err(e)) => Err(format!("cookies: {e}")),
                    Some(Ok(f)) => video::lire_infos(&o, &u, Some(f.chemin())),
                    None => video::lire_infos(&o, &u, None),
                }
            };
            let _ = tx.send(r);
        });
        self.modale = Some(Modale::Video {
            url,
            rx: Some(rx),
            infos: None,
            choix: self.settings.video_quality,
            de: String::new(),
            a: String::new(),
            debut: false,
            sortie: self.settings.video_options.clone(),
        });
    }

    /// Playlist / chaîne : vidéos lues dans un fil (`debut` = déjà lues, « Charger plus »).
    fn lire_liste(
        &self,
        url: &str,
        debut: usize,
        stop: &Arc<AtomicBool>,
    ) -> Receiver<Result<video::Playlist, String>> {
        let (tx, rx) = std::sync::mpsc::channel();
        let o = self.outils.clone();
        let jarre = self.jarres.get(url).cloned();
        let lien = video::liste(url).map_or_else(|| url.to_string(), |l| l.1);
        let stop = stop.clone();
        std::thread::spawn(move || {
            let r = if o.ytdlp.is_none() {
                Err(video::ABSENT.to_string())
            } else {
                match jarre.map(|j| crate::cookies::Fichier::creer(&crate::cookies::dossier(), &j))
                {
                    Some(Err(e)) => Err(format!("cookies: {e}")),
                    Some(Ok(f)) => video::lire_liste(&o, &lien, Some(f.chemin()), debut, &stop),
                    None => video::lire_liste(&o, &lien, None, debut, &stop),
                }
            };
            let _ = tx.send(r);
        });
        rx
    }

    fn ouvrir_liste(&mut self, url: String) {
        let stop = Arc::new(AtomicBool::new(false));
        let rx = self.lire_liste(&url, 0, &stop);
        self.modale = Some(Modale::Liste {
            url,
            rx: Some(rx),
            stop,
            liste: None,
            erreur_suite: None,
            coches: Vec::new(),
            filtre: String::new(),
            choix: self.settings.video_quality,
            sortie: self.settings.video_options.clone(),
            dossier: true,
        });
    }

    /// Une entrée par vidéo cochée (même chemin qu'une vidéo seule : moteur multi-connexions,
    /// ajouts), sans doublon d'une vidéo déjà dans la file ; renvoie le nombre ajouté.
    /// Sous-titres : langue de l'interface (absente : rien), comme l'ajout direct.
    fn ajouter_liste(
        &mut self,
        url: &str,
        p: &video::Playlist,
        coches: &[bool],
        qualite: video::Qualite,
        sortie: &video::Sortie,
        dossier: bool,
    ) -> usize {
        let mut dir = PathBuf::from(self.dossier_tri(video_ou_son(qualite)));
        if dossier {
            dir.push(video::nom_dossier(&p.titre));
        }
        let dir = dir.to_string_lossy().into_owned();
        // Cookies de la page de la liste (playlist privée à soi, vidéos réservées aux membres).
        let jarre = self.jarres.get(url).cloned();
        let options = video::Sortie {
            sub_lang: Some(self.settings.lang.code().into()),
            sub_auto: false,
            ..sortie.clone()
        };
        let mut n = 0;
        for (v, _) in p.videos.iter().zip(coches).filter(|(_, c)| **c) {
            let deja = self.entries.iter().any(|e| {
                e.url == v.url
                    && match &e.status {
                        Status::Done => e
                            .name
                            .as_ref()
                            .is_some_and(|n| Path::new(&e.dir).join(n).exists()),
                        s => *s != Status::Canceled,
                    }
            });
            if deja {
                continue;
            }
            if let Some(j) = &jarre {
                self.jarres.insert(v.url.clone(), j.clone());
            }
            let e = self.nouvelle(
                v.url.clone(),
                Kind::Video {
                    quality: qualite,
                    estimate: None,
                    section: None,
                    from_start: false,
                    login: jarre.is_some(),
                    options: options.clone(),
                },
            );
            e.dir = dir.clone();
            e.name = Some(v.titre.clone());
            n += 1;
        }
        n
    }

    /// Page vidéo envoyée par l'extension (bouton près du lecteur, menu) : choix du format,
    /// ou ajout direct dans la qualité par défaut si la fenêtre est réduite. Déjà dans la file
    /// (pas finie) : cookies neufs gardés et reprise (`--continue`) ; c'est aussi ainsi qu'une
    /// vidéo à connexion reprend après un redémarrage de l'appli (cookies jamais sur le disque).
    fn ajouter_video(
        &mut self,
        url: String,
        cookies: Option<Vec<crate::cookies::Cookie>>,
        reduite: bool,
    ) -> Statut {
        let url = url.trim().to_string();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Statut::Refuse;
        }
        let jarre = cookies
            .map(|c| crate::cookies::pour_la_page(&url, c))
            .filter(|c| !c.is_empty());
        if let Some(j) = jarre {
            self.jarres.insert(url.clone(), Arc::new(j));
        }
        if let Some(e) = self.entries.iter_mut().rev().find(|e| {
            e.url == url
                && matches!(e.kind, Kind::Video { .. })
                && !matches!(e.status, Status::Done | Status::Canceled)
        }) {
            if matches!(e.status, Status::Paused | Status::Failed(_)) {
                e.status = Status::Queued;
            }
            self.selected = Some(e.id);
            self.tab = Tab::Downloads;
            return Statut::DejaActif;
        }
        // Une liste demande toujours ses cases (pas d'ajout direct de centaines de vidéos).
        if reduite
            && video::liste(&url).is_none()
            && self.outils.ytdlp.is_some()
            && (cfg!(not(windows)) || self.outils.complets())
        {
            let login = self.jarres.contains_key(&url);
            self.nouvelle(
                url,
                Kind::Video {
                    quality: self.settings.video_quality,
                    estimate: None,
                    section: None,
                    from_start: false,
                    login,
                    // Sous-titres du site dans la langue de l'interface (absents : rien).
                    options: video::Sortie {
                        sub_lang: Some(self.settings.lang.code().into()),
                        ..self.settings.video_options.clone()
                    },
                },
            );
            return Statut::Ajoute;
        }
        self.ouvrir_video(url);
        Statut::Ajoute
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
        let dossiers: Vec<String> = apres::CATEGORIES
            .iter()
            .map(|&c| self.dossier_categorie(c))
            .collect();
        let e = &mut self.entries[i];
        e.status = Status::Running;
        // Limites : globale (partagée) puis celle du téléchargement (index 1, voir `schedule`).
        let ctl = Arc::new(Control::avec_limites(vec![
            self.globale.clone(),
            Arc::new(Seau::new(mo_s(e.speed_limit_mb))),
        ]));
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
                // Tri par type : nom pas encore choisi, dossier par défaut (pas un dossier choisi
                // pour cette entrée) ; le moteur choisit d'après le nom donné par la sonde.
                let tri = (self.settings.sort_by_type
                    && e.name.is_none()
                    && e.dir == self.settings.folder)
                    .then(|| {
                        engine::Tri(Arc::new(move |nom: &str| {
                            PathBuf::from(&dossiers[apres::categorie(nom) as usize])
                        }))
                    });
                let opts = Options {
                    connections: e.connections,
                    expected_sha256: e.sha256.clone(),
                    headers,
                    tri,
                    ..Default::default()
                };
                std::thread::spawn(move || engine::run(&url, &dir, name.as_deref(), &opts, &c))
            }
            Kind::Video {
                quality,
                estimate,
                section,
                from_start,
                login,
                options,
            } => {
                let o = self.outils.clone();
                let n = e.connections;
                let tache = video::Tache {
                    qualite: quality,
                    estimation: estimate,
                    section,
                    debut: from_start,
                    sortie: options,
                };
                let jarre = self.jarres.get(&url).cloned();
                std::thread::spawn(move || {
                    // Cookies en mémoire seulement : après un redémarrage, il faut les renvoyer.
                    let f = match (jarre, login) {
                        (None, true) => return Err(video::ERR_COOKIES.to_string()),
                        (None, false) => None,
                        (Some(j), _) => Some(
                            crate::cookies::Fichier::creer(&crate::cookies::dossier(), &j)
                                .map_err(|e| format!("cookies: {e}"))?,
                        ),
                    };
                    // `f` vit jusqu'à la fin de yt-dlp, puis le fichier est effacé.
                    let f = f.as_ref().map(|f| f.chemin());
                    video::telecharger(&o, &url, &dir, &tache, f, n, &c)
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
        let mut a_extraire = Vec::new();
        if let Some((id, v)) = self.limite_choisie.take() {
            if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
                e.speed_limit_mb = v;
                self.last_save = f64::NEG_INFINITY;
            }
        }
        for e in &mut self.entries {
            let Some(j) = self.jobs.get_mut(&e.id) else {
                continue;
            };
            // Limite propre changée en direct (sans effet sur un torrent).
            if let Some(s) = j.ctl.limites.get(1) {
                s.regler(mo_s(e.speed_limit_mb));
            }
            let l = j.ctl.snapshot();
            j.sample(now, &l);
            if l.file_name.is_some() {
                e.name = l.file_name.clone();
            }
            if let Some(d) = &l.dir {
                e.dir = d.to_string_lossy().into_owned();
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
                Some(Ok(Outcome::Done(p))) => {
                    // Nom final (« nom (1).ext » si un fichier du même nom est apparu entre-temps).
                    if e.kind == Kind::Http {
                        if let Some(n) = p.file_name() {
                            e.name = Some(n.to_string_lossy().into_owned());
                        }
                    }
                    // Marque « venu d'Internet » (Windows) ; un dossier seulement pour un torrent
                    // (yt-dlp sans chemin final renvoie le dossier de téléchargement).
                    if p.is_file() || matches!(e.kind, Kind::Torrent { .. }) {
                        crate::motw::marquer(&p, &e.url, e.referrer.as_deref());
                    }
                    // Cookies inutiles une fois fini : ils ne restent pas dans queue.json.
                    e.cookies = None;
                    // Décompression automatique (zip / 7z, fichier HTTP seulement).
                    if self.settings.auto_extract
                        && e.kind == Kind::Http
                        && e.name
                            .as_deref()
                            .and_then(apres::format)
                            .is_some_and(|f| f != apres::Format::Rar)
                    {
                        a_extraire.push(e.id);
                    }
                    self.fini_session = true;
                    Status::Done
                }
                // Mis en pause par l'heure d'arrêt : repart à la prochaine heure de démarrage.
                Some(Ok(Outcome::Stopped)) => {
                    match self.plan_suspendus.iter().position(|&i| i == e.id) {
                        Some(k) => {
                            self.plan_suspendus.swap_remove(k);
                            Status::Queued
                        }
                        None => Status::Paused,
                    }
                }
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
        // Cookies d'une page dont plus rien n'attend : oubliés.
        let ouverte = match &self.modale {
            Some(
                Modale::Video { url, .. }
                | Modale::Outils { url, .. }
                | Modale::Liste { url, .. }
                | Modale::VideoOuListe(url),
            ) => Some(url.as_str()),
            _ => None,
        };
        let entries = &self.entries;
        self.jarres.retain(|u, _| {
            ouverte == Some(u.as_str())
                || entries.iter().any(|e| {
                    &e.url == u
                        && matches!(e.kind, Kind::Video { .. })
                        && !matches!(e.status, Status::Done | Status::Canceled)
                })
        });
        for id in a_extraire {
            self.lancer_outil(id, Outil::Extraire);
        }
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
            let e = &self.entries[i];
            // Petit fichier de taille connue : passe devant, sans attendre les gros.
            let petit = e.size.is_some_and(|s| s.saturating_sub(e.done) < PETIT);
            // Planification : tout attend l'heure de démarrage (ou « Démarrer maintenant »).
            if e.status == Status::Queued && !self.plan_arme && (actifs < MAX_ACTIFS || petit) {
                self.start(i);
                actifs += usize::from(!petit);
            }
        }
    }

    /// Extraction ou empreinte d'un fichier fini dans un fil (une seule à la fois par outil).
    fn lancer_outil(&mut self, id: u64, o: Outil) {
        let Some(f) = self
            .entries
            .iter()
            .find(|e| e.id == id)
            .and_then(|e| Some(Path::new(&e.dir).join(e.name.as_ref()?)))
        else {
            return;
        };
        if self.taches.contains_key(&(id, o)) {
            return;
        }
        let suivi = Arc::new(apres::Suivi::default());
        let (tx, rx) = std::sync::mpsc::channel();
        let s = suivi.clone();
        let effacer = self.settings.extract_delete;
        std::thread::spawn(move || {
            let r = match o {
                Outil::Extraire => apres::extraire(&f, &s).map(|d| {
                    // Archive gardée par défaut ; effacée seulement après une extraction réussie.
                    if effacer {
                        if let Err(e) = std::fs::remove_file(&f) {
                            eprintln!("{}: {e}", f.display());
                        }
                    }
                    d.to_string_lossy().into_owned()
                }),
                Outil::Sha => apres::sha256(&f, &s),
            };
            let _ = tx.send(r);
        });
        self.resultats.remove(&(id, o));
        self.taches.insert((id, o), TacheFichier { suivi, rx });
    }

    /// Outils des fichiers finis : résultats des fils, clic du panneau de détail.
    fn suivre_outils(&mut self) {
        let finies: Vec<_> = self
            .taches
            .iter()
            .filter_map(|(k, t)| match t.rx.try_recv() {
                Ok(r) => Some((*k, r)),
                Err(TryRecvError::Disconnected) => Some((*k, Err("panic".into()))),
                Err(TryRecvError::Empty) => None,
            })
            .collect();
        for (k, r) in finies {
            self.taches.remove(&k);
            // Arrêté : rien à montrer (l'extraction partielle est effacée).
            if r.as_ref().err().map(String::as_str) != Some(apres::ARRETE) {
                self.resultats.insert(k, r);
            }
        }
        if let Some((id, o)) = self.outil_demande.take() {
            match self.taches.get(&(id, o)) {
                Some(t) => t.suivi.stop.store(true, Ordering::Relaxed),
                None => self.lancer_outil(id, o),
            }
        }
    }

    /// Fenêtre qui attend un résultat (fil, délai) : il faut repasser de temps en temps.
    fn modale_attend(&self) -> bool {
        matches!(
            self.modale,
            Some(
                Modale::Video { rx: Some(_), .. }
                    | Modale::Liste { rx: Some(_), .. }
                    | Modale::Torrent { rx: Some(_), .. }
                    | Modale::Outils { .. }
                    | Modale::Appairage { .. }
                    | Modale::FinCompte { .. }
            )
        )
    }

    /// Torrent fini qui partage encore.
    fn partage(&self, id: u64) -> bool {
        self.jobs.get(&id).is_some_and(|j| j.ctl.lock().seeding)
    }

    /// Heure de démarrage passée : la file part ; heure d'arrêt : ce qui tourne se met en pause
    /// et attend le prochain démarrage.
    fn planifier_file(&mut self) {
        let m = crate::systeme::minute_locale();
        let s = &self.settings;
        let (arme, pause) =
            plan::planifier(self.plan_arme, self.plan_vu, m, s.plan_start, s.plan_stop);
        (self.plan_arme, self.plan_vu) = (arme, m);
        if !pause {
            return;
        }
        for e in &self.entries {
            if e.status != Status::Running || self.partage(e.id) {
                continue;
            }
            if let Some(j) = self.jobs.get(&e.id).filter(|j| j.handle.is_some()) {
                j.ctl.request_stop();
                self.plan_suspendus.push(e.id);
            }
        }
    }

    /// Tout est fini (rien en échec ni en pause, au moins un fini pendant cette session) :
    /// fermer fluxdl, ou compte à rebours avant la veille / l'arrêt du PC (fenêtre devant).
    /// Compte à rebours écoulé : file et réglages écrits, puis la commande du système.
    fn action_de_fin(&mut self, ctx: &egui::Context) {
        if let Some(Modale::FinCompte { eteindre, depuis }) = self.modale {
            if self.now - depuis >= COMPTE_FIN_S {
                self.modale = None;
                // Une seule fois : il faut la rechoisir pour la prochaine fois.
                self.settings.end_action = Fin::Rien;
                if self.capture.is_none() {
                    store::save_queue(&store::data_dir(), QUEUE, &self.entries, &self.inconnues);
                    store::save(SETTINGS, &self.settings);
                }
                (self.executer)(eteindre);
            }
            return;
        }
        let action = self.settings.end_action;
        // Une extraction ou une empreinte en cours n'est pas coupée par la veille ou l'arrêt.
        if action == Fin::Rien
            || self.modale.is_some()
            || !self.taches.is_empty()
            || !plan::fin_atteinte(self.entries.iter().map(|e| &e.status), self.fini_session)
        {
            return;
        }
        self.fini_session = false;
        if action == Fin::Fermer {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        self.modale = Some(Modale::FinCompte {
            eteindre: action == Fin::Eteindre,
            depuis: self.now,
        });
        montrer(ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
            egui::UserAttentionType::Critical,
        ));
    }

    fn persist(&mut self, now: f64) {
        if self.capture.is_none() && now - self.last_save > 2.0 {
            store::save_queue(&store::data_dir(), QUEUE, &self.entries, &self.inconnues);
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
                // Fenêtre au second plan : l'astuce ne tourne pas (aucune image dépensée).
                let devant = ui.ctx().input(|i| i.viewport().focused.unwrap_or(true));
                if self.capture.is_none() && devant && self.now - depuis >= ASTUCE_S {
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
                // Images seulement pendant le fondu (15 i/s) ; sinon une seule, au prochain
                // changement.
                if self.capture.is_none() && devant {
                    let reste = (self.now - depuis).max(0.0);
                    if reste < FONDU_S {
                        ui.ctx().request_repaint_after(Duration::from_millis(66));
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
        // Débit total en direct et limite globale, à droite du titre ; clic = menu des limites
        // (illimitée, 1… 50 Mo/s, personnalisée).
        let (mut total, mut n) = (0.0, 0);
        for e in self.entries.iter().filter(|e| e.status == Status::Running) {
            n += 1;
            total += self.jobs.get(&e.id).map_or(0.0, |j| j.speed);
        }
        let titre_w = ui
            .painter()
            .layout_no_wrap(t(l, "Downloads").into(), affiche(30.0), TEXTE)
            .size()
            .x;
        let p = ui.painter().clone();
        let g = (n > 0)
            .then(|| p.layout_no_wrap(format!("{}/s", octets(total as u64)), demi(14.0), TEXTE));
        let gl = p.layout_no_wrap(
            format!("{} ▾", texte_limite(l, self.settings.speed_limit_mb)),
            moyen(13.0),
            TITRE,
        );
        let mut ga = (n > 0).then(|| {
            p.layout_no_wrap(
                format!("{n} {}", compte(l, n, "active")),
                regulier(12.5),
                TEXTE3,
            )
        });
        let vitesse_w = g.as_ref().map_or(0.0, |g| 40.0 + g.size().x + 14.0);
        let limite_w = 16.0 + 20.0 + gl.size().x + 16.0;
        // Colonne étroite : « 2 actives » s'efface avant de toucher le titre.
        let mut larg = vitesse_w + ga.as_ref().map_or(0.0, |a| a.size().x + 10.0) + limite_w;
        if r.right() - larg < r.left() + titre_w + 16.0 {
            larg -= ga.take().map_or(0.0, |a| a.size().x + 10.0);
        }
        let b = Rect::from_min_size(
            pos2(r.right() - larg, r.center().y - 17.0),
            vec2(larg, 34.0),
        );
        let rep = ui
            .interact(b, egui::Id::unique("limite_globale"), Sense::click())
            .on_hover_text(t(l, "Speed limit for all downloads"));
        let h = survol(ui, &rep);
        if h > 0.0 {
            p.add(lueur(
                b.expand(3.0),
                teinte(0.5).gamma_multiply(0.5 * h),
                10.0,
            ));
        }
        p.add(verre(ui.ctx(), b, 17, 0.7 + 0.2 * h));
        p.add(reflet(b.shrink(1.0), 16, 0.6));
        let mut x = b.left();
        if let Some(g) = g {
            orbe(&p, pos2(x + 22.0, b.center().y), 9.0, GEL_VERT);
            icone(
                &p,
                Rect::from_center_size(pos2(x + 22.0, b.center().y), Vec2::splat(11.0)),
                Icone::Telecharger,
                Color32::WHITE,
            );
            p.galley(
                pos2(x + 40.0, b.center().y - g.size().y / 2.0),
                g.clone(),
                TEXTE,
            );
            x += 40.0 + g.size().x + 10.0;
            if let Some(ga) = ga {
                p.galley(
                    pos2(x, b.center().y - ga.size().y / 2.0),
                    ga.clone(),
                    TEXTE3,
                );
                x += ga.size().x + 10.0;
            }
            // Séparateur entre le débit et la limite.
            p.vline(x, b.y_range().shrink(8.0), Stroke::new(1.0, nuit(40)));
        }
        icone(
            &p,
            Rect::from_center_size(pos2(x + 24.0, b.center().y), Vec2::splat(14.0)),
            Icone::Eclair,
            TITRE,
        );
        p.galley(pos2(x + 36.0, b.center().y - gl.size().y / 2.0), gl, TITRE);
        if let Some(v) = popup_limite(&rep, l, self.settings.speed_limit_mb) {
            self.settings.speed_limit_mb = v;
        }
        ui.add_space(18.0);
        self.paste_card(ui);
        ui.add_space(20.0);
        self.bandeau_plan(ui);
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

    /// File en attente de l'heure de démarrage : « Démarre à 02:00 » et « Démarrer maintenant ».
    fn bandeau_plan(&mut self, ui: &mut Ui) {
        let l = self.settings.lang;
        let Some(debut) = self.settings.plan_start.filter(|_| self.plan_arme) else {
            return;
        };
        let maintenant = carte(ui, Margin::symmetric(14, 10), |ui| {
            let w = ui.available_width();
            ui.set_width(w);
            let (r, _) = ui.allocate_exact_size(vec2(w, 36.0), Sense::hover());
            let p = ui.painter();
            orbe(p, pos2(r.left() + 16.0, r.center().y), 14.0, GEL_BLEU);
            icone_relief(
                p,
                Rect::from_center_size(pos2(r.left() + 16.0, r.center().y), Vec2::splat(14.0)),
                Icone::Pause,
            );
            let titre = format!("{} {}", t(l, "Starts at"), plan::hhmm(debut));
            let aide = match self.settings.plan_stop {
                Some(f) => format!(
                    "{} {}.",
                    t(l, "Downloads wait until then. Pause at"),
                    plan::hhmm(f)
                ),
                None => t(l, "Downloads wait until then.").into(),
            };
            texte(
                ui,
                pos2(r.left() + 40.0, r.center().y),
                Align2::LEFT_CENTER,
                &titre,
                demi(15.0),
                TITRE,
            );
            let txt = t(l, "Start now");
            let bw = ui
                .painter()
                .layout_no_wrap(txt.into(), demi(13.5), TEXTE)
                .size()
                .x
                + 64.0;
            let b = Rect::from_min_size(pos2(r.right() - bw, r.center().y - 18.0), vec2(bw, 36.0));
            let clic = a_la_place(ui, b, |ui| {
                bouton_gel(ui, txt, Some(Icone::Lecture), b.size(), GEL_VERT)
            })
            .clicked();
            // Aide sous le titre, renvoyée à la ligne (colonne étroite).
            ui.add_space(4.0);
            ui.label(egui::RichText::new(aide).font(regulier(12.5)).color(TEXTE2));
            clic
        });
        if maintenant {
            self.plan_arme = false;
        }
        ui.add_space(14.0);
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
            let vide = self.url.trim().is_empty();
            let go = a_la_place(ui, bd, |ui| {
                ui.add_enabled_ui(!vide, |ui| {
                    bouton_gel(
                        ui,
                        t(l, "Download"),
                        Some(Icone::Telecharger),
                        bd.size(),
                        GEL_VERT,
                    )
                })
                .inner
            });
            if (go.clicked() || entree) && !vide {
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
        ui.add_space(24.0);
        // Sur une carte de verre : texte bleu nuit lisible (le blanc ombré sur l'eau claire
        // n'avait qu'un contraste de 1,6:1).
        carte(ui, Margin::symmetric(24, 22), |ui| {
            ui.set_width(ui.available_width());
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
                ui.label(
                    egui::RichText::new(t(l, "No downloads yet"))
                        .font(demi(17.0))
                        .color(TITRE),
                );
                ui.label(
                    egui::RichText::new(t(
                        l,
                        "Paste a link above. Each file is split across up to 32 parallel connections.",
                    ))
                    .font(moyen(13.5))
                    .color(TEXTE),
                );
            });
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
                Status::Failed(m) if m == engine::ERR_EXPIRE => &[
                    (Icone::Lien, "Update the link", Act::NouveauLien),
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
            Act::Navigateur => {
                ouvrir_url(&self.entries[i].url);
            }
            // Un torrent qui partage s'arrête « fini » (voir torrent::telecharger).
            Act::Pause => match self.jobs.get(&id).filter(|j| j.handle.is_some()) {
                Some(j) => j.ctl.request_stop(),
                None => {
                    self.entries[i].status = Status::Paused;
                    self.last_save = f64::NEG_INFINITY;
                }
            },
            Act::Resume => {
                // Même contrôle qu'au lancement : un .part effacé à la main n'est pas recréé.
                let e = &mut self.entries[i];
                e.status = Status::Queued;
                store::au_lancement(e);
            }
            Act::NouveauLien => {
                if self.modale.is_none() {
                    self.modale = Some(Modale::NouveauLien {
                        id,
                        lien: String::new(),
                    });
                }
            }
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
            store::save_queue(&store::data_dir(), QUEUE, &self.entries, &self.inconnues);
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
        if e.status == Status::Done && !matches!(e.kind, Kind::Torrent { .. }) {
            self.detail_outils(ui, e);
        }
        // Limite propre au téléchargement (en plus de la globale) ; torrents : globale seulement.
        ui.add_space(14.0);
        if matches!(e.kind, Kind::Torrent { .. }) {
            ui.label(
                egui::RichText::new(t(
                    l,
                    "Speed limit: torrents follow the global limit only (shared by all torrents).",
                ))
                .font(regulier(12.5))
                .color(TEXTE3),
            );
        } else {
            if let Some(v) = menu_limite(ui, l, e.speed_limit_mb, 30.0) {
                self.limite_choisie.set(Some((e.id, v)));
            }
            ui.label(
                egui::RichText::new(t(l, "this download, on top of the global limit"))
                    .font(regulier(12.5))
                    .color(TEXTE3),
            );
            // yt-dlp seul (HLS, son, passage…) : `--limit-rate` fixé à son lancement.
            let ytdlp = matches!(e.kind, Kind::Video { .. })
                && e.status == Status::Running
                && self.jobs.get(&e.id).is_some_and(|j| !j.ctl.lock().ranges);
            if ytdlp {
                ui.label(
                    egui::RichText::new(t(
                        l,
                        "Downloaded by yt-dlp: a new limit applies at its next start (pause, then resume).",
                    ))
                    .font(regulier(12.0))
                    .color(ATTENTION),
                );
            }
        }
        let Some(j) = self.jobs.get(&e.id) else {
            ui.add_space(28.0);
            ui.label(
                egui::RichText::new(match e.status {
                    Status::Done => t(l, "Download complete."),
                    Status::Queued if self.plan_arme => t(l, "Waiting for the scheduled start."),
                    Status::Queued => t(
                        l,
                        "Waiting: it starts as soon as one of the running downloads finishes.",
                    ),
                    _ => t(l, "No live data: resume to see the connections."),
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
            (
                lv.size_unverified && e.kind == Kind::Http,
                "Size not verifiable: the server did not announce it.",
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
        // Vidéo en liens directs (moteur, `ranges`) : carte et connexions comme un fichier.
        if matches!(e.kind, Kind::Video { .. }) && !lv.ranges {
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

    /// Fichier fini : extraire une archive zip / 7z, calculer son SHA-256 (copie, comparaison
    /// avec une empreinte collée).
    fn detail_outils(&self, ui: &mut Ui, e: &Entry) {
        let l = self.settings.lang;
        let Some(n) = e.name.as_deref() else {
            return;
        };
        let existe = Path::new(&e.dir).join(n).is_file();
        let vu =
            |o| self.taches.contains_key(&(e.id, o)) || self.resultats.contains_key(&(e.id, o));
        if !existe && !vu(Outil::Extraire) && !vu(Outil::Sha) {
            return;
        }
        let w = ui.available_width();
        ui.add_space(16.0);
        self.section(ui, t(l, "File"), "");
        ui.add_space(6.0);
        let demande = |o| self.outil_demande.set(Some((e.id, o)));
        // Panneau étroit : le 2e bouton passe à la ligne.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            match apres::format(n) {
                Some(apres::Format::Rar) => {
                    ui.label(
                        egui::RichText::new(t(l, "RAR archives are not supported."))
                            .font(regulier(12.5))
                            .color(TEXTE3),
                    );
                }
                Some(_) if existe => {
                    let r = if self.taches.contains_key(&(e.id, Outil::Extraire)) {
                        bouton_secondaire(
                            ui,
                            t(l, "Stop extracting"),
                            Some(Icone::Fermer),
                            vec2(184.0, 32.0),
                        )
                    } else {
                        bouton_principal(
                            ui,
                            t(l, "Extract"),
                            Some(Icone::Dossier),
                            vec2(112.0, 32.0),
                        )
                    };
                    if r.clicked() {
                        demande(Outil::Extraire);
                    }
                }
                _ => {}
            }
            if existe {
                let (m, i) = if self.taches.contains_key(&(e.id, Outil::Sha)) {
                    (t(l, "Stop SHA-256"), Icone::Fermer)
                } else {
                    (t(l, "Compute SHA-256"), Icone::Comparatif)
                };
                if bouton_secondaire(ui, m, Some(i), vec2(176.0, 32.0)).clicked() {
                    demande(Outil::Sha);
                }
            }
        });
        // Avancement d'un fil : jauge et pourcentage.
        let avancement = |ui: &mut Ui, titre: &str, tf: &TacheFichier| {
            let fait = tf.suivi.fait.load(Ordering::Relaxed);
            let total = tf.suivi.total.load(Ordering::Relaxed).max(1);
            let frac = (fait as f64 / total as f64).min(1.0) as f32;
            ui.add_space(8.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, 16.0), Sense::hover());
            texte(
                ui,
                r.left_center(),
                Align2::LEFT_CENTER,
                titre,
                regulier(12.5),
                TEXTE2,
            );
            texte(
                ui,
                r.right_center(),
                Align2::RIGHT_CENTER,
                &format!("{:.0}%", frac * 100.0),
                regulier(12.5),
                TEXTE2,
            );
            let (r, _) = ui.allocate_exact_size(vec2(w, 10.0), Sense::hover());
            jauge(ui.painter(), r, frac, ACCENT, true);
        };
        let message = |ui: &mut Ui, m: &str, c: Color32| {
            ui.add_space(8.0);
            ui.add(egui::Label::new(egui::RichText::new(m).font(regulier(12.5)).color(c)).wrap());
        };
        if let Some(tf) = self.taches.get(&(e.id, Outil::Extraire)) {
            avancement(ui, t(l, "Extracting…"), tf);
        }
        match self.resultats.get(&(e.id, Outil::Extraire)) {
            Some(Ok(d)) => {
                message(ui, &format!("{} {d}", t(l, "Extracted to:")), SUCCES);
                ui.add_space(4.0);
                if bouton_secondaire(
                    ui,
                    t(l, "Open folder"),
                    Some(Icone::Dossier),
                    vec2(150.0, 30.0),
                )
                .clicked()
                {
                    open_folder(Path::new(d));
                }
            }
            Some(Err(m)) => message(ui, crate::i18n::tr(l, m), DANGER),
            None => {}
        }
        if let Some(tf) = self.taches.get(&(e.id, Outil::Sha)) {
            avancement(ui, t(l, "Computing SHA-256…"), tf);
        }
        match self.resultats.get(&(e.id, Outil::Sha)) {
            Some(Ok(h)) => {
                ui.add_space(8.0);
                // Empreinte entière, sur deux lignes au besoin.
                let g = ui
                    .painter()
                    .layout(h.clone(), regulier(12.0), TEXTE, w - 38.0);
                let (r, _) = ui.allocate_exact_size(vec2(w, g.size().y.max(30.0)), Sense::hover());
                let bc = Rect::from_min_size(pos2(r.right() - 30.0, r.top()), vec2(30.0, 30.0));
                ui.painter()
                    .galley(pos2(r.left(), r.center().y - g.size().y / 2.0), g, TEXTE);
                if bouton_icone_ici(
                    ui,
                    bc,
                    egui::Id::unique("copier-sha"),
                    Icone::Copier,
                    t(l, "Copy"),
                )
                .clicked()
                {
                    ui.ctx().copy_text(h.clone());
                }
                ui.add_space(6.0);
                let (r, _) = ui.allocate_exact_size(vec2(w, 30.0), Sense::hover());
                let pr = Rect::from_min_size(r.min, vec2((w - 120.0).max(120.0), 30.0));
                let mut colle = self.sha_colle.borrow_mut();
                champ(
                    ui,
                    pr,
                    &mut colle,
                    t(l, "Paste the expected SHA-256"),
                    None,
                    regulier(12.0),
                );
                if !colle.trim().is_empty() {
                    let (m, c) = if apres::meme_empreinte(&colle, h) {
                        (t(l, "Match"), SUCCES)
                    } else {
                        (t(l, "Mismatch"), DANGER)
                    };
                    pastille(ui, pos2(pr.right() + 10.0, r.center().y), m, c);
                }
            }
            Some(Err(m)) => message(ui, crate::i18n::tr(l, m), DANGER),
            None => {}
        }
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
                        t(l, "Shared by all downloads, also in the main window. Off means no limit."),
                        |ui| {
                            let mut on = self.settings.speed_limit_mb > 0.0;
                            if interrupteur(ui, &mut on).changed() {
                                self.settings.speed_limit_mb = if on { 50.0 } else { 0.0 };
                            }
                            if on {
                                ui.add_space(8.0);
                                let v = &mut self.settings.speed_limit_mb;
                                boite_valeur(ui, &format!("{v:.0} {}/s", octets_parts(1_000_000).1), 84.0);
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
                self.reglages_apres(ui);
                ui.add_space(20.0);
                self.reglages_plan(ui);
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

    /// Carte « Après le téléchargement » : tri par type (un dossier par catégorie), extraction
    /// automatique des archives.
    fn reglages_apres(&mut self, ui: &mut Ui) {
        let l = self.settings.lang;
        carte(ui, Margin::symmetric(20, 6), |ui| {
            ui.set_width(ui.available_width());
            titre_section(ui, t(l, "After download"));
            reglage(
                ui,
                t(l, "Sort by file type"),
                t(
                    l,
                    "New files and videos go to the folder of their type. Torrents are not sorted.",
                ),
                |ui| {
                    interrupteur(ui, &mut self.settings.sort_by_type);
                },
            );
            if self.settings.sort_by_type {
                for c in apres::CATEGORIES {
                    let defaut = Path::new(&self.settings.folder)
                        .join(t(l, c.nom()))
                        .to_string_lossy()
                        .into_owned();
                    let actuel = self.dossier_categorie(c);
                    separateur(ui);
                    reglage(ui, t(l, c.nom()), &actuel, |ui| {
                        if bouton_secondaire(
                            ui,
                            t(l, "Browse"),
                            Some(Icone::Dossier),
                            vec2(112.0, 36.0),
                        )
                        .clicked()
                            && self.picker.is_none()
                        {
                            self.picker = Some(choose_folder(&actuel));
                            self.picker_cat = Some(c);
                        }
                        let (r, _) = ui.allocate_exact_size(vec2(250.0, 36.0), Sense::hover());
                        let d = self.settings.sort_folders.entry(c).or_default();
                        champ(ui, r, d, &defaut, None, regulier(13.0));
                    });
                }
            }
            separateur(ui);
            reglage(
                ui,
                t(l, "Extract archives automatically"),
                t(l, "zip and 7z, into a folder named after the archive, next to it. RAR is not supported."),
                |ui| {
                    interrupteur(ui, &mut self.settings.auto_extract);
                },
            );
            separateur(ui);
            reglage(
                ui,
                t(l, "Delete the archive after extraction"),
                t(
                    l,
                    "Only once the extraction succeeded. Off: the archive is kept.",
                ),
                |ui| {
                    interrupteur(ui, &mut self.settings.extract_delete);
                },
            );
        });
    }

    /// Carte « Planification » : heure de démarrage de la file, heure de pause, action de fin.
    fn reglages_plan(&mut self, ui: &mut Ui) {
        let l = self.settings.lang;
        // Heure (minutes depuis minuit) : interrupteur, valeur, curseur par pas de 5 min.
        let heure = |ui: &mut Ui, v: &mut Option<u16>, defaut: u16| {
            let mut on = v.is_some();
            let mut change = interrupteur(ui, &mut on).changed();
            if change {
                *v = on.then_some(defaut);
            }
            if let Some(m) = v {
                ui.add_space(8.0);
                boite_valeur(ui, &plan::hhmm(*m), 64.0);
                let mut f = *m as f64;
                if curseur(ui, &mut f, (0.0, 1435.0), false, 5.0, 200.0).changed() {
                    *m = ((f / 5.0).round() * 5.0) as u16;
                    change = true;
                }
            }
            change
        };
        let mut change = false;
        carte(ui, Margin::symmetric(20, 6), |ui| {
            ui.set_width(ui.available_width());
            titre_section(ui, t(l, "Scheduling"));
            reglage(
                ui,
                t(l, "Start the queue at"),
                t(
                    l,
                    "Downloads wait until this time (local time), then start.",
                ),
                |ui| change |= heure(ui, &mut self.settings.plan_start, 120),
            );
            if self.settings.plan_start.is_some() {
                separateur(ui);
                reglage(
                    ui,
                    t(l, "Pause at"),
                    t(
                        l,
                        "Running downloads pause and wait for the next start time.",
                    ),
                    |ui| change |= heure(ui, &mut self.settings.plan_stop, 480),
                );
            }
            separateur(ui);
            reglage(
                ui,
                t(l, "When everything is done"),
                t(
                    l,
                    "Sleep and shut down: 60 s countdown you can cancel. Never after a failed or paused download; not kept after a restart.",
                ),
                |_| {},
            );
            // Quatre choix : sur leur propre ligne, sous l'aide (trop larges à côté).
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                segmente(
                    ui,
                    &mut self.settings.end_action,
                    &[
                        (Fin::Rien, t(l, "Nothing")),
                        (Fin::Fermer, t(l, "Quit fluxdl")),
                        (Fin::Veille, t(l, "Sleep")),
                        (Fin::Eteindre, t(l, "Shut down")),
                    ],
                );
            });
            ui.add_space(12.0);
        });
        if self.settings.plan_start.is_none() {
            self.settings.plan_stop = None;
        }
        // Heures changées : la file attend (ou non) selon la nouvelle plage.
        if change {
            self.plan_vu = crate::systeme::minute_locale();
            self.plan_arme = plan::arme_au_depart(
                self.plan_vu,
                self.settings.plan_start,
                self.settings.plan_stop,
            );
        }
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
        let vues: [(Tab, Lang, &str); 32] = [
            (Tab::Downloads, Lang::En, "downloads"),
            (Tab::Benchmark, Lang::En, "benchmark"),
            (Tab::Settings, Lang::En, "settings"),
            (Tab::Downloads, Lang::Fr, "downloads_fr"),
            (Tab::Downloads, Lang::Fr, "update_banner_fr"),
            (Tab::Settings, Lang::Fr, "settings_extension_fr"),
            (Tab::Downloads, Lang::Fr, "video_formats_fr"),
            (Tab::Downloads, Lang::Fr, "video_live_fr"),
            (Tab::Downloads, Lang::En, "video_options_en"),
            (Tab::Settings, Lang::Fr, "settings_video_fr"),
            (Tab::Downloads, Lang::Fr, "torrent_pieces_fr"),
            (Tab::Downloads, Lang::Fr, "video_tools_fr"),
            (Tab::Settings, Lang::Fr, "extension_guide_fr"),
            (Tab::Settings, Lang::Fr, "extension_guide_firefox_fr"),
            (Tab::Settings, Lang::En, "extension_guide_en"),
            (Tab::Settings, Lang::Fr, "pairing_fr"),
            (Tab::Downloads, Lang::Fr, "extension_bubble_fr"),
            (Tab::Downloads, Lang::Fr, "cancel_fr"),
            (Tab::Downloads, Lang::Fr, "expired_link_fr"),
            (Tab::Downloads, Lang::Fr, "non_latin_names_fr"),
            (Tab::Downloads, Lang::Fr, "playlist_fr"),
            (Tab::Downloads, Lang::En, "playlist_choice_en"),
            (Tab::Downloads, Lang::Fr, "playlist_private_fr"),
            (Tab::Downloads, Lang::Fr, "lotD_downloads_fr"),
            (Tab::Settings, Lang::Fr, "lotD_settings_plan_fr"),
            (Tab::Settings, Lang::En, "lotD_settings_plan_en"),
            (Tab::Downloads, Lang::Fr, "lotD_countdown_fr"),
            (Tab::Settings, Lang::Fr, "lotE_settings_fr"),
            (Tab::Settings, Lang::En, "lotE_settings_en"),
            (Tab::Downloads, Lang::Fr, "lotE_detail_fr"),
            (Tab::Downloads, Lang::En, "lotE_progress_en"),
            (Tab::Downloads, Lang::Fr, "narrow_900x600_fr"),
        ];
        for (i, (tab, lang, nom)) in vues.into_iter().enumerate() {
            let k = 30 * (i as u32 + 1);
            if f == k - 24 {
                self.tab = tab;
                self.settings.lang = lang;
                self.demo_vue(nom);
                if nom.starts_with("narrow") {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(900.0, 600.0)));
                }
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
        self.plan_arme = nom == "lotD_downloads_fr";
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
                signature: f("fluxdl.exe.sig", 129),
            })),
            "settings_extension_fr" => self.defile = Some(300.0),
            "video_formats_fr" => {
                // ffmpeg présent : le passage « de … à … » est saisissable.
                self.outils.ffmpeg = Some(PathBuf::from("ffmpeg"));
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
                        en_direct: false,
                        sous_titres: demo_sous_titres(),
                    })),
                    choix: video::Qualite::P1080,
                    de: "00:02:00".into(),
                    a: "00:05:30".into(),
                    debut: false,
                    sortie: video::Sortie {
                        subtitles: true,
                        sub_lang: Some("fr".into()),
                        thumbnail: true,
                        chapters: true,
                        ..Default::default()
                    },
                });
            }
            "video_options_en" => {
                // Sous-titres automatiques en .srt, MP4.
                self.outils.ffmpeg = Some(PathBuf::from("ffmpeg"));
                self.modale = Some(Modale::Video {
                    url: "https://www.youtube.com/watch?v=aqz-KE-bpKQ".into(),
                    rx: None,
                    infos: Some(Ok(video::Infos {
                        titre: "Big Buck Bunny 60fps 4K - Official Blender Foundation Short Film"
                            .into(),
                        duree: Some(634.0),
                        hauteur_max: 2160,
                        choix: vec![
                            (video::Qualite::Best, true, Some(2_431_000_000)),
                            (video::Qualite::P1080, true, Some(402_000_000)),
                            (video::Qualite::P720, true, Some(188_000_000)),
                            (video::Qualite::M4a, true, Some(10_300_000)),
                            (video::Qualite::Mp3, true, Some(10_300_000)),
                        ],
                        en_direct: false,
                        sous_titres: demo_sous_titres(),
                    })),
                    choix: video::Qualite::Best,
                    de: String::new(),
                    a: String::new(),
                    debut: false,
                    sortie: video::Sortie {
                        subtitles: true,
                        sub_lang: Some("de".into()),
                        sub_auto: true,
                        srt: true,
                        thumbnail: true,
                        chapters: false,
                        mp4: true,
                    },
                });
            }
            "video_live_fr" => {
                self.outils.ffmpeg = None;
                let u = "https://www.youtube.com/live/jfKfPfyJRdk";
                // Cookies de démonstration (vides) : montre la mention « avec vos cookies ».
                self.jarres.insert(u.into(), Arc::new(Vec::new()));
                self.modale = Some(Modale::Video {
                    url: u.into(),
                    rx: None,
                    infos: Some(Ok(video::Infos {
                        titre: "Live : concert en direct (réservé aux membres)".into(),
                        duree: None,
                        hauteur_max: 1080,
                        choix: vec![
                            (video::Qualite::Best, true, None),
                            (video::Qualite::P1080, true, None),
                            (video::Qualite::P720, true, None),
                            (video::Qualite::M4a, true, None),
                            (video::Qualite::Mp3, true, None),
                        ],
                        en_direct: true,
                        sous_titres: Vec::new(),
                    })),
                    choix: video::Qualite::Best,
                    de: String::new(),
                    a: String::new(),
                    debut: true,
                    sortie: video::Sortie::default(),
                });
            }
            "playlist_fr" | "playlist_private_fr" => {
                self.outils.ffmpeg = Some(PathBuf::from("ffmpeg"));
                let titres = [
                    ("Coding Challenge 1: Starfield Simulation", 834.0),
                    ("Coding Challenge #2: Menger Sponge Fractal", 841.0),
                    ("Coding Challenge 3: The Snake Game", 1647.0),
                    ("Coding Challenge #4: Purple Rain in Processing", 563.0),
                    ("Coding Challenge #5: Space Invaders in JavaScript", 2481.0),
                    ("Coding Challenge #6: Mitosis Simulation with p5.js", 1072.0),
                    ("Coding Challenge #7: Solar System in Processing - Part 1 (2D)", 1325.0),
                    ("Coding Challenge #8: Solar System in Processing - Part 2 (3D)", 1166.0),
                ];
                let videos: Vec<_> = titres
                    .iter()
                    .enumerate()
                    .map(|(i, (titre, d))| video::VideoListe {
                        url: format!("https://www.youtube.com/watch?v=demo{i}"),
                        titre: (*titre).into(),
                        duree: Some(*d),
                        numero: i + 1,
                    })
                    .collect();
                let mut coches = vec![true; videos.len()];
                coches[3] = false;
                let liste = if nom == "playlist_fr" {
                    Ok(video::Playlist {
                        titre: "Coding Challenges".into(),
                        videos,
                        suite: true,
                        lues: 500,
                    })
                } else {
                    Err(video::ERR_LISTE_PRIVEE.into())
                };
                self.modale = Some(Modale::Liste {
                    url: "https://www.youtube.com/playlist?list=PLRqwX-V7Uu6ZiZxtDDRCi6uhfTH4FilpH"
                        .into(),
                    rx: None,
                    stop: Arc::default(),
                    liste: Some(liste),
                    erreur_suite: None,
                    coches,
                    filtre: String::new(),
                    choix: video::Qualite::P1080,
                    sortie: video::Sortie {
                        subtitles: true,
                        thumbnail: true,
                        mp4: true,
                        ..Default::default()
                    },
                    dossier: true,
                });
            }
            "playlist_choice_en" => {
                self.modale = Some(Modale::VideoOuListe(
                    "https://www.youtube.com/watch?v=17WoOqgXsRM&list=PLRqwX-V7Uu6ZiZxtDDRCi6uhfTH4FilpH"
                        .into(),
                ))
            }
            "settings_video_fr" => self.defile = Some(1150.0),
            "lotD_downloads_fr" => {
                self.settings.speed_limit_mb = 5.0;
                self.settings.plan_start = Some(120);
                self.settings.plan_stop = Some(480);
                self.entries[0].speed_limit_mb = 10.0;
            }
            "lotD_settings_plan_fr" | "lotD_settings_plan_en" => {
                self.settings.end_action = Fin::Eteindre;
                self.defile = Some(300.0);
            }
            "lotD_countdown_fr" => {
                self.modale = Some(Modale::FinCompte {
                    eteindre: true,
                    depuis: self.now - 18.0,
                })
            }
            "lotE_settings_fr" | "lotE_settings_en" => {
                self.settings.sort_by_type = true;
                self.settings.auto_extract = true;
                let images = Path::new(&self.settings.folder).join("Photos");
                self.settings
                    .sort_folders
                    .insert(Categorie::Images, images.to_string_lossy().into_owned());
                self.defile = Some(if nom.ends_with("en") { 760.0 } else { 330.0 });
            }
            "lotE_detail_fr" | "lotE_progress_en" => {
                // Archive finie présente sur le disque (boutons montrés seulement si elle existe).
                let d = std::env::temp_dir().join("fluxdl-capture-lotE");
                let _ = std::fs::create_dir_all(&d);
                let _ = std::fs::write(d.join("blender-4.2.3-windows-x64.zip"), b"PK");
                if let Some(e) = self.entries.iter_mut().find(|e| e.id == 2) {
                    e.dir = d.to_string_lossy().into_owned();
                    e.name = Some("blender-4.2.3-windows-x64.zip".into());
                }
                self.selected = Some(2);
                self.taches.clear();
                self.resultats.clear();
                let h = "3f1c2a9e0b7d4e8f6a5c1b2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f";
                if nom == "lotE_detail_fr" {
                    let x = d.join("blender-4.2.3-windows-x64");
                    self.resultats
                        .insert((2, Outil::Extraire), Ok(x.to_string_lossy().into_owned()));
                    self.resultats.insert((2, Outil::Sha), Ok(h.into()));
                    *self.sha_colle.borrow_mut() = h.to_uppercase();
                } else {
                    for (o, fait) in [(Outil::Extraire, 0.62), (Outil::Sha, 0.35)] {
                        let suivi = Arc::new(apres::Suivi::default());
                        suivi.total.store(381_000_000, Ordering::Relaxed);
                        suivi.fait.store((381e6 * fait) as u64, Ordering::Relaxed);
                        let (tx, rx) = std::sync::mpsc::channel();
                        // Capture : le fil n'existe pas, la tâche reste « en cours ».
                        std::mem::forget(tx);
                        self.taches.insert((2, o), TacheFichier { suivi, rx });
                    }
                }
            }
            "torrent_pieces_fr" => self.selected = Some(5),
            "cancel_fr" => {
                self.modale = Some(Modale::Annuler {
                    id: 3,
                    supprimer: true,
                })
            }
            "expired_link_fr" => {
                if let Some(e) = self.entries.iter_mut().find(|e| e.id == 4) {
                    e.status = Status::Failed(engine::ERR_EXPIRE.into());
                }
                self.selected = Some(4);
                self.modale = Some(Modale::NouveauLien {
                    id: 4,
                    lien: "https://cdn.example.org/archive.tar.zst?token=…".into(),
                });
            }
            "non_latin_names_fr" => {
                self.entries[1].name = Some("Документы_2024 — отчёт.zip".into());
                self.entries[2].name = Some("Ελληνικά → αρχείο.pdf".into());
                self.selected = Some(2);
            }

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
                    etat: EtatOutils::Pret(Box::new(video::Paquet {
                        ytdlp: Some((f("yt-dlp.exe", 23_627_865), String::new())),
                        ffmpeg: Some((f("ffmpeg.zip", 92_386_792), String::new())),
                        deno: Some((f("deno.zip", 42_917_472), String::new())),
                    })),
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
            speed_limit_mb: 0.0,
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
            section: None,
            from_start: false,
            login: false,
            options: video::Sortie::default(),
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
    /// Lien expiré : en coller un nouveau (le `.part` est gardé si c'est le même fichier).
    NouveauLien,
    Cancel,
    Remove,
}

impl eframe::App for App {
    /// File, demandes de l'extension, presse-papiers, mises à jour, enregistrement : appelé
    /// avant chaque `ui`, et seul quand la fenêtre est réduite ou masquée (eframe n'y dessine
    /// plus rien) : les téléchargements avancent donc aussi fenêtre réduite.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Horloge à nous : fenêtre masquée, l'heure d'egui reste celle de la dernière image.
        let now = self.horloge.elapsed().as_secs_f64();
        UNITES_FR.store(self.settings.lang == Lang::Fr, Ordering::Relaxed);
        // Limite globale changée en direct ; torrents : limite de leur session (à part).
        let globale = mo_s(self.settings.speed_limit_mb);
        self.globale.regler(globale);
        if let Some(m) = torrent::lance() {
            m.limiter(globale);
        }
        if self.capture.is_none() {
            self.planifier_file();
            self.schedule(now);
        }
        self.now = now;
        if self.capture.is_none() {
            self.action_de_fin(ctx);
        }
        if self.bench.handle.as_ref().is_some_and(|h| h.is_finished()) {
            let h = self.bench.handle.take().map(|h| h.join());
            self.bench.result = h.map(|r| r.unwrap_or_else(|_| Err("panic".into())));
            self.bench.shown = now;
        }
        self.suivre_outils();
        if let Some(rx) = &self.picker {
            match rx.try_recv() {
                Ok(d) => {
                    match self.picker_cat.take() {
                        Some(c) => {
                            self.settings.sort_folders.insert(c, d);
                        }
                        None => self.settings.folder = d,
                    }
                    self.picker = None;
                }
                Err(TryRecvError::Disconnected) => {
                    self.picker = None;
                    self.picker_cat = None;
                }
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
        let reduite = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        for d in recus {
            // 2e instance lancée sans lien : montrer la fenêtre.
            if d.url.is_empty() {
                montrer(ctx);
                continue;
            }
            let avant = self.modale.is_some();
            let mut ajout = || {
                if d.video() {
                    return self.ajouter_video(d.url.clone(), d.video_cookies.clone(), reduite);
                }
                self.ajouter(
                    d.url.clone(),
                    d.referrer.clone(),
                    d.cookies.clone(),
                    d.locale,
                )
            };
            match &d.decision {
                // Décidé sous le verrou : une demande abandonnée (délai passé) n'est pas ajoutée.
                Some(dec) => dec.decider(ajout),
                None => {
                    ajout();
                }
            }
            // Un choix à faire (formats, fichiers d'un torrent) : la fenêtre passe devant.
            if !avant && self.modale.is_some() {
                montrer(ctx);
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
            self.suivre_appairage(ctx);
        }
        self.suivre_modale();
        // Nom chinois, japonais ou coréen : police du système chargée à la première rencontre.
        if !self.cjk {
            let torrent = match &self.modale {
                Some(Modale::Torrent {
                    apercu: Some(Ok(a)),
                    ..
                }) => a.fichiers.iter().any(|f| besoin_cjk(&f.nom)) || besoin_cjk(&a.nom),
                _ => false,
            };
            if torrent
                || self
                    .entries
                    .iter()
                    .any(|e| e.name.as_deref().is_some_and(besoin_cjk))
            {
                self.cjk = true;
                charger_cjk(ctx);
            }
        }
        self.persist(now);
        // Au repos, aucune image : on ne repasse que si quelque chose bouge ou télécharge
        // (les animations egui demandent elles-mêmes leurs images).
        let vivant =
            self.entries.iter().any(|e| {
                e.status == Status::Running || e.status == Status::Queued && !self.plan_arme
            }) || !self.orphelins.is_empty()
                || self.bench.handle.is_some()
                || self.picker.is_some()
                || self.picker_torrent.is_some()
                || !self.taches.is_empty()
                || matches!(
                    self.maj.etat(),
                    maj::Etat::Recherche | maj::Etat::Telechargement(..)
                )
                || self.modale_attend();
        if vivant {
            ctx.request_repaint_after(Duration::from_millis(250));
        } else if self.plan_arme {
            // File en attente de l'heure de démarrage : un passage toutes les 20 s suffit.
            ctx.request_repaint_after(Duration::from_secs(20));
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
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
        // Fenêtre étroite (petit écran à 125–150 %) : la liste prend la place du détail.
        if self.tab == Tab::Downloads && ecran.width() >= 1100.0 {
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
        // Textes copiés par l'appli (Ctrl+C, bouton Copier) : à ne pas proposer.
        if let Some(c) = ctx.output(|o| {
            o.commands.iter().rev().find_map(|c| match c {
                egui::OutputCommand::CopyText(t) => Some(t.trim().to_string()),
                _ => None,
            })
        }) {
            self.copie_app = Some(c);
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
        store::save_queue(&store::data_dir(), QUEUE, &self.entries, &self.inconnues);
        store::save(SETTINGS, &self.settings);
        if let Some(exe) = &self.relance {
            maj::relancer(exe);
        }
    }
}

/// Fenêtre réduite ou derrière les autres : la remettre devant.
fn montrer(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
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
/// Catégorie du tri par type d'une vidéo : son seul → Musique.
fn video_ou_son(q: video::Qualite) -> Categorie {
    if q.audio() {
        Categorie::Musique
    } else {
        Categorie::Videos
    }
}

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

use crate::systeme::{ouvrir_dossier as open_folder, ouvrir_url};

pub fn octets(n: u64) -> String {
    let (v, u) = octets_parts(n);
    format!("{v} {u}")
}

/// Unités françaises (Mo, Go, virgule décimale) : langue de l'interface, mise à jour à chaque
/// passe (`octets` est appelé partout sans la langue).
static UNITES_FR: AtomicBool = AtomicBool::new(false);

/// Nombre et unité séparés (le nombre en grand, l'unité en discret).
fn octets_parts(n: u64) -> (String, &'static str) {
    let fr = UNITES_FR.load(Ordering::Relaxed);
    let n = n as f64;
    let (v, i) = match n {
        n if n >= 1e9 => (format!("{:.2}", n / 1e9), 3),
        n if n >= 1e6 => (format!("{:.1}", n / 1e6), 2),
        n if n >= 1e3 => (format!("{:.0}", n / 1e3), 1),
        n => (format!("{n:.0}"), 0),
    };
    if fr {
        (v.replace('.', ","), ["o", "ko", "Mo", "Go"][i])
    } else {
        (v, ["B", "kB", "MB", "GB"][i])
    }
}

/// Mo/s → octets/s (0 = illimitée).
fn mo_s(v: f32) -> u64 {
    (v.max(0.0) * 1e6) as u64
}

/// « 5 Mo/s » ou « Illimitée ».
fn texte_limite(l: Lang, v: f32) -> String {
    if v > 0.0 {
        format!("{v:.0} {}/s", octets_parts(1_000_000).1)
    } else {
        t(l, "Unlimited").into()
    }
}

/// Bouton « Limite : 5 Mo/s ▾ » et son menu ; nouvelle valeur (Mo/s) si elle change.
fn menu_limite(ui: &mut Ui, l: Lang, v: f32, hauteur: f32) -> Option<f32> {
    let txt = format!("{} {} ▾", t(l, "Limit:"), texte_limite(l, v));
    let w = ui
        .painter()
        .layout_no_wrap(txt.clone(), demi(13.5), TEXTE)
        .size()
        .x
        + 52.0;
    let rep = bouton_secondaire(ui, &txt, Some(Icone::Eclair), vec2(w, hauteur));
    popup_limite(&rep, l, v)
}

/// Menu des limites sous `rep` (illimitée, 1, 5, 10, 50 Mo/s, personnalisée) ; nouvelle
/// valeur (Mo/s) si elle change.
fn popup_limite(rep: &Response, l: Lang, v: f32) -> Option<f32> {
    let mut nv = v;
    egui::Popup::menu(rep)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(260.0);
            for c in LIMITES {
                let r = ui.selectable_label(nv == c, texte_limite(l, c));
                if r.clicked() {
                    nv = c;
                    ui.close();
                }
            }
            ui.separator();
            ui.label(
                egui::RichText::new(t(l, "Custom"))
                    .font(moyen(13.0))
                    .color(TEXTE2),
            );
            ui.horizontal(|ui| {
                let mut f = if nv > 0.0 { nv as f64 } else { 20.0 };
                boite_valeur(ui, &texte_limite(l, f as f32), 84.0);
                if curseur(ui, &mut f, (1.0, 1000.0), true, 1.0, 150.0).changed() {
                    nv = f.round() as f32;
                }
            });
        });
    (nv != v).then_some(nv)
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
    // Contrôle vide (ex. yt-dlp sous Linux) : la ligne suivante reste sous celle-ci.
    ui.advance_cursor_after_rect(r);
}

/// Sous-titres des captures de la fenêtre des formats.
fn demo_sous_titres() -> Vec<video::SousTitre> {
    [
        ("en", "English", false),
        ("fr", "Français", false),
        ("de", "Deutsch", true),
        ("en", "English", true),
        ("es", "Español", true),
        ("fr", "Français", true),
    ]
    .into_iter()
    .map(|(c, n, auto)| video::SousTitre {
        code: c.into(),
        nom: n.into(),
        auto,
    })
    .collect()
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// Faux yt-dlp qui rend une playlist → une entrée par vidéo cochée, dans le sous-dossier
    /// au nom nettoyé, sans doublon, et pas plus de `MAX_ACTIFS` lancées à la fois.
    #[test]
    fn liste_dans_la_file() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("fluxdl-test-file-liste-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let e: Vec<String> = (1..=5)
            .map(|i| {
                format!(r#"{{"url":"https://www.youtube.com/watch?v=v{i}","title":"Vidéo {i}","duration":6{i}}}"#)
            })
            .collect();
        std::fs::write(
            d.join("liste.json"),
            format!(
                r#"{{"title":"Ma liste: été/2026","entries":[{}]}}"#,
                e.join(",")
            ),
        )
        .unwrap();
        // Liste lue ; téléchargement : échec immédiat (rien ne sort sur Internet).
        let yt = d.join("yt-dlp");
        std::fs::write(
            &yt,
            format!(
                "#!/bin/sh\ncase \"$*\" in\n*--flat-playlist*) cat '{}/liste.json' ;;\n*) exit 1 ;;\nesac\n",
                d.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&yt, std::fs::Permissions::from_mode(0o755)).unwrap();
        let o = video::Outils {
            ytdlp: Some(yt),
            ..Default::default()
        };
        let url = "https://www.youtube.com/playlist?list=PLx";
        let p = video::lire_liste(&o, url, None, 0, &AtomicBool::new(false)).unwrap();
        assert_eq!(p.videos.len(), 5);

        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, Some(d.join("cap")), None);
        ctx.run_ui(Default::default(), |_| {})
            .textures_delta
            .clear();
        app.capture = None;
        app.entries.clear();
        app.jobs.clear();
        app.outils = o;
        app.settings.folder = d.join("dl").to_string_lossy().into_owned();
        let sortie = video::Sortie {
            subtitles: true,
            thumbnail: true,
            ..Default::default()
        };
        let mut ajoute = |coches: [bool; 5], dossier: bool| {
            app.ajouter_liste(url, &p, &coches, video::Qualite::P720, &sortie, dossier)
        };
        assert_eq!(ajoute([false, true, false, false, false], true), 1);
        // La 2e est déjà dans la file : pas en double.
        assert_eq!(ajoute([true, true, true, false, true], true), 3);
        assert_eq!(ajoute([true; 5], false), 1);
        assert_eq!(ajoute([true; 5], true), 0);
        let urls: Vec<_> = app.entries.iter().map(|e| e.url.as_str()).collect();
        assert_eq!(
            urls,
            [2, 1, 3, 5, 4].map(|i| format!("https://www.youtube.com/watch?v=v{i}"))
        );
        let sous = d.join("dl").join("Ma liste_ été_2026");
        let e = &app.entries[0];
        assert_eq!(e.dir, sous.to_string_lossy());
        assert_eq!(e.name.as_deref(), Some("Vidéo 2"));
        assert_eq!(e.status, Status::Queued);
        assert_eq!(
            e.kind,
            Kind::Video {
                quality: video::Qualite::P720,
                estimate: None,
                section: None,
                from_start: false,
                login: false,
                options: video::Sortie {
                    sub_lang: Some(app.settings.lang.code().into()),
                    ..sortie.clone()
                },
            }
        );
        // Sans sous-dossier : le dossier de téléchargement.
        assert_eq!(app.entries[4].dir, app.settings.folder);
        // Le nombre de téléchargements simultanés est respecté.
        app.schedule(1.0);
        let n = |s: Status| app.entries.iter().filter(|e| e.status == s).count();
        assert_eq!((n(Status::Running), n(Status::Queued)), (MAX_ACTIFS, 2));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 0.2.9 : tri par type désactivé → dossier par défaut ; activé → dossier choisi pour la
    /// catégorie, sinon `<dossier>/<catégorie traduite>` ; vidéo → Vidéos, son seul → Musique,
    /// playlist en sous-dossier ; fichier HTTP : dossier par défaut (trié au nom de la sonde).
    #[test]
    fn dossier_selon_le_tri() {
        let d = std::env::temp_dir().join(format!("fluxdl-test-tri-{}", std::process::id()));
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, Some(d.join("cap")), None);
        ctx.run_ui(Default::default(), |_| {})
            .textures_delta
            .clear();
        app.capture = None;
        app.entries.clear();
        app.jobs.clear();
        app.settings.folder = d.join("dl").to_string_lossy().into_owned();
        app.settings.lang = Lang::Fr;
        let video = |q| Kind::Video {
            quality: q,
            estimate: None,
            section: None,
            from_start: false,
            login: false,
            options: Default::default(),
        };
        let dl = |n: &str| d.join("dl").join(n).to_string_lossy().into_owned();
        let dir = app
            .nouvelle("https://v/1".into(), video(video::Qualite::P720))
            .dir
            .clone();
        assert_eq!(dir, app.settings.folder);
        app.settings.sort_by_type = true;
        let dir = app
            .nouvelle("https://v/2".into(), video(video::Qualite::P720))
            .dir
            .clone();
        assert_eq!(dir, dl("Vidéos"));
        let dir = app
            .nouvelle("https://v/3".into(), video(video::Qualite::Mp3))
            .dir
            .clone();
        assert_eq!(dir, dl("Musique"));
        let dir = app
            .nouvelle(
                "https://t/4".into(),
                Kind::Torrent {
                    hash: "h".into(),
                    files: None,
                    started: false,
                },
            )
            .dir
            .clone();
        assert_eq!(dir, app.settings.folder);
        assert_eq!(app.dossier_tri(Categorie::Autres), dl("Autres"));
        app.settings.lang = Lang::En;
        assert_eq!(app.dossier_tri(Categorie::Programmes), dl("Programs"));
        app.settings
            .sort_folders
            .insert(Categorie::Images, "/photos".into());
        app.settings
            .sort_folders
            .insert(Categorie::Documents, "  ".into());
        assert_eq!(app.dossier_tri(Categorie::Images), "/photos");
        assert_eq!(app.dossier_tri(Categorie::Documents), dl("Documents"));
        let p = video::Playlist {
            titre: "Ma liste".into(),
            videos: vec![video::VideoListe {
                url: "https://www.youtube.com/watch?v=a".into(),
                titre: "A".into(),
                duree: None,
                numero: 1,
            }],
            suite: false,
            lues: 1,
        };
        let o = video::Sortie::default();
        app.ajouter_liste(
            "https://www.youtube.com/playlist?list=x",
            &p,
            &[true],
            video::Qualite::Best,
            &o,
            true,
        );
        let e = app.entries.last().unwrap();
        assert_eq!(
            e.dir,
            d.join("dl")
                .join("Videos")
                .join("Ma liste")
                .to_string_lossy()
        );
        // Fichier HTTP : dossier par défaut à l'ajout, le moteur trie au nom de la sonde.
        app.ajouter("https://h/a.zip".into(), None, None, false);
        assert_eq!(app.entries.last().unwrap().dir, app.settings.folder);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 0.2.9 : action de fin jamais au lancement ni après un échec ou une pause ; compte à
    /// rebours de 60 s puis la commande (fausse ici : jamais de vraie veille ni d'arrêt).
    #[test]
    fn action_de_fin_apres_compte_a_rebours() {
        static APPELS: Mutex<Vec<bool>> = Mutex::new(Vec::new());
        let d = std::env::temp_dir().join(format!("fluxdl-test-fin-{}", std::process::id()));
        let ctx = egui::Context::default();
        // Mode capture gardé : rien n'est écrit dans le vrai dossier de données.
        let mut app = App::new(&ctx, Some(d.join("cap")), None);
        ctx.run_ui(Default::default(), |_| {})
            .textures_delta
            .clear();
        app.executer = |e| APPELS.lock().unwrap().push(e);
        app.jobs.clear();
        app.modale = None;
        for e in &mut app.entries {
            e.status = Status::Done;
        }
        app.settings.end_action = Fin::Eteindre;
        app.now = 100.0;
        // Tout est fini dès le lancement : rien.
        app.action_de_fin(&ctx);
        assert!(app.modale.is_none());
        app.fini_session = true;
        for autre in [Status::Failed("x".into()), Status::Paused] {
            app.entries[0].status = autre;
            app.action_de_fin(&ctx);
            assert!(app.modale.is_none());
        }
        app.entries[0].status = Status::Done;
        app.action_de_fin(&ctx);
        assert!(matches!(
            app.modale,
            Some(Modale::FinCompte { eteindre: true, .. })
        ));
        app.now += 59.0;
        app.action_de_fin(&ctx);
        assert!(APPELS.lock().unwrap().is_empty());
        app.now += 1.5;
        app.action_de_fin(&ctx);
        assert_eq!(*APPELS.lock().unwrap(), [true]);
        assert!(app.modale.is_none());
        // Une seule fois : l'action est remise à « Rien ».
        assert_eq!(app.settings.end_action, Fin::Rien);
        let _ = std::fs::remove_dir_all(&d);
    }
}
