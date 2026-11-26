//! Fenêtre egui : barre latérale, file de téléchargements, panneau de détail par connexion,
//! comparatif, réglages.

mod draw;

use crate::i18n::t;
use crate::store::{self, Entry, Lang, Settings, Status};
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
    cancel: bool,
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
            cancel: false,
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
    /// Bouton « Coller » : collage demandé à cet instant (0 = aucun).
    paste_wait: f64,
    /// Sélecteur de dossier ouvert dans un fil.
    picker: Option<Receiver<String>>,
}

impl App {
    pub fn new(ctx: &egui::Context, capture: Option<PathBuf>) -> Self {
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
            paste_wait: 0.0,
            picker: None,
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
        for e in &mut app.entries {
            // Interrompu par un arrêt ou un plantage : reprend tout seul.
            if e.status == Status::Running {
                e.status = Status::Queued;
            }
        }
        app.next_id = app.entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        app
    }

    fn add(&mut self) {
        let url = self.url.trim().to_string();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return;
        }
        let sha = self.sha.trim().to_string();
        self.entries.push(Entry {
            id: self.next_id,
            url,
            dir: self.settings.folder.clone(),
            name: None,
            sha256: (!sha.is_empty()).then_some(sha),
            connections: self.settings.connections,
            status: Status::Queued,
            size: None,
            done: 0,
        });
        self.selected = Some(self.next_id);
        self.born.insert(self.next_id, self.now);
        self.next_id += 1;
        self.url.clear();
        self.sha.clear();
    }

    fn start(&mut self, i: usize) {
        let e = &mut self.entries[i];
        e.status = Status::Running;
        let ctl = Arc::new(Control::default());
        let opts = Options {
            connections: e.connections,
            speed_limit: (self.settings.speed_limit_mb.max(0.0) * 1e6) as u64,
            expected_sha256: e.sha256.clone(),
            ..Default::default()
        };
        let (url, dir, name, c) = (
            e.url.clone(),
            PathBuf::from(&e.dir),
            e.name.clone(),
            ctl.clone(),
        );
        let h = std::thread::spawn(move || engine::run(&url, &dir, name.as_deref(), &opts, &c));
        self.jobs.insert(e.id, Job::new(ctl, Some(h)));
    }

    /// Fin des téléchargements terminés, démarrage des suivants, noms et tailles à jour.
    fn schedule(&mut self, now: f64) {
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
            if !j.handle.as_ref().is_some_and(|h| h.is_finished()) {
                continue;
            }
            let r = j
                .handle
                .take()
                .map(|h| h.join().unwrap_or_else(|_| Err("panic".into())));
            j.speed = 0.0;
            e.status = match r {
                Some(Ok(Outcome::Done(_))) => Status::Done,
                Some(Ok(Outcome::Stopped)) if j.cancel => {
                    delete_parts(e);
                    Status::Canceled
                }
                Some(Ok(Outcome::Stopped)) => Status::Paused,
                Some(Err(m)) => Status::Failed(m),
                None => e.status.clone(),
            };
        }
        let mut actifs = self
            .entries
            .iter()
            .filter(|e| e.status == Status::Running)
            .count();
        for i in 0..self.entries.len() {
            if actifs < MAX_ACTIFS && self.entries[i].status == Status::Queued {
                self.start(i);
                actifs += 1;
            }
        }
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
                let fond = ui.painter().add(egui::Shape::Noop);
                let cadre = egui::Frame::NONE
                    .inner_margin(Margin::same(14))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
                        tuile_icone(ui.painter(), r, Icone::Eclair, GEL_VERT);
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(t(l, "Done early? Take half."))
                                .font(demi(13.5))
                                .color(TITRE),
                        );
                        ui.add_space(-2.0);
                        let m = t(
                            l,
                            "A connection that finishes early takes half of the largest remaining range.",
                        );
                        ui.label(egui::RichText::new(m).font(regulier(12.5)).color(TEXTE2));
                    });
                // Encart : verre plus clair, posé dans le verre de la barre.
                let r = cadre.response.rect;
                ui.painter().set(
                    fond,
                    egui::Shape::Vec(vec![
                        vdeg(r, 12, blanc(215), blanc(150)),
                        reflet(r.shrink(1.0), 11, 0.5),
                        egui::Shape::rect_stroke(r, 12, Stroke::new(1.0, nuit(45)), StrokeKind::Inside),
                    ]),
                );
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
        );
        if let Some(s) = sous {
            let (rs, _) = ui.allocate_exact_size(vec2(r.width(), 22.0), Sense::hover());
            texte_ciel(
                ui.painter(),
                rs.left_center(),
                Align2::LEFT_CENTER,
                s,
                moyen(14.0),
            );
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
                self.paste_wait = self.now;
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
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
            let sha = Rect::from_min_size(
                pos2(st.left() - 10.0 - lc - 12.0 - 132.0, r.top()),
                vec2(132.0, 30.0),
            );
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
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
            texte_ciel(
                ui.painter(),
                r.center(),
                Align2::CENTER_CENTER,
                t(l, "No downloads yet"),
                demi(17.0),
            );
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
            texte_ciel(
                ui.painter(),
                r.center(),
                Align2::CENTER_CENTER,
                t(
                    l,
                    "Paste a link above. Each file is split across up to 32 parallel connections.",
                ),
                moyen(13.5),
            );
        });
    }

    fn entry_card(&self, ui: &mut Ui, e: &Entry) -> Option<Act> {
        let l = self.settings.lang;
        let job = self.jobs.get(&e.id);
        let (etat, ce) = self.status_label(&e.status);
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
            tuile_fichier(&p, tuile, TypeFichier::de(&nom));
            let (x0, x1) = (tuile.right() + 14.0, r.right() - 16.0);
            let actes: &[(Icone, &str, Act)] = match e.status {
                Status::Running | Status::Queued => &[
                    (Icone::Pause, "Pause", Act::Pause),
                    (Icone::Dossier, "Open folder", Act::Folder),
                    (Icone::Fermer, "Cancel", Act::Cancel),
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
                (Status::Failed(m), _) => m.clone(),
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
                if let (Some(s), true) = (e.size, j.speed > 1.0) {
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

    fn status_label(&self, s: &Status) -> (&'static str, Color32) {
        let l = self.settings.lang;
        match s {
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
            Act::Pause => match self.jobs.get(&id).filter(|j| j.handle.is_some()) {
                Some(j) => j.ctl.request_stop(),
                None => self.entries[i].status = Status::Paused,
            },
            Act::Resume => self.entries[i].status = Status::Queued,
            Act::Cancel => match self.jobs.get_mut(&id).filter(|j| j.handle.is_some()) {
                Some(j) => {
                    j.cancel = true;
                    j.ctl.request_stop();
                }
                None => {
                    delete_parts(&self.entries[i]);
                    self.entries[i].status = Status::Canceled;
                }
            },
            Act::Remove => {
                self.entries.remove(i);
                self.born.remove(&id);
                self.jobs.remove(&id);
                if self.selected == Some(id) {
                    self.selected = None;
                }
            }
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
        tuile_fichier(ui.painter(), tuile, TypeFichier::de(&nom));
        let x0 = tuile.right() + 14.0;
        let g = ligne(ui, nom, demi(16.5), TEXTE, r.right() - x0);
        ui.painter()
            .galley(pos2(x0, r.top() + 12.0 - g.size().y / 2.0 + 1.0), g, TEXTE);
        let (etat, ce) = self.status_label(&e.status);
        let pr = pastille(ui, pos2(x0, r.top() + 35.0), etat, ce);
        let hote = e
            .url
            .split("://")
            .nth(1)
            .unwrap_or(&e.url)
            .split('/')
            .next()
            .unwrap_or("");
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
        let avis: Vec<&str> = [
            (lv.restarted, "Remote file changed: restarted from zero."),
            (
                !lv.ranges && e.status == Status::Running,
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
                )
            });
        ui.add_space(20.0);
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
                connexions(ui, &lv, &[], t(l, "stolen"), t(l, "Done early? Take half."));
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
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
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
                });
                ui.add_space(20.0);
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
        let vues: [(u32, Tab, Lang, &str); 4] = [
            (30, Tab::Downloads, Lang::En, "downloads"),
            (60, Tab::Benchmark, Lang::En, "benchmark"),
            (90, Tab::Settings, Lang::En, "settings"),
            (120, Tab::Downloads, Lang::Fr, "downloads_fr"),
        ];
        for (k, tab, lang, nom) in vues {
            if f == k - 24 {
                self.tab = tab;
                self.settings.lang = lang;
            }
            if f == k {
                ctx.send_viewport_cmd(screenshot(dir.join(format!("{nom}.png"))));
            }
        }
        if f > 135 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
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
        self.selected = Some(1);
        self.bench.url = "https://proof.ovh.net/files/100Mb.dat".into();
        self.bench.result = Some(Ok((11.4e6, 86.9e6)));
        self.bench.shown = -10.0;
        self.settings.speed_limit_mb = 50.0;
        self.next_id = 5;
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
    Folder,
    Pause,
    Resume,
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
        // « Coller » : le texte arrive par un événement Paste une image plus tard.
        if self.paste_wait > 0.0 {
            let colle = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Paste(s) => Some(s.trim().to_string()),
                    _ => None,
                })
            });
            if let Some(s) = colle {
                self.url = s;
                self.paste_wait = 0.0;
            } else if now - self.paste_wait > 1.0 {
                self.paste_wait = 0.0;
            }
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
        self.persist(now);
        // Au repos, aucune image : on ne redessine que si quelque chose bouge ou télécharge
        // (les animations egui demandent elles-mêmes leurs images).
        let vivant = self
            .entries
            .iter()
            .any(|e| matches!(e.status, Status::Running | Status::Queued))
            || self.bench.handle.is_some()
            || self.picker.is_some()
            || self.paste_wait > 0.0;
        if vivant {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.capture.is_some() {
            return;
        }
        // Arrêt propre : l'état de chaque téléchargement est sauvé, il reprendra au lancement.
        for j in self.jobs.values() {
            j.ctl.request_stop();
        }
        let fin = std::time::Instant::now() + Duration::from_secs(2);
        while self
            .jobs
            .values()
            .any(|j| j.handle.as_ref().is_some_and(|h| !h.is_finished()))
            && std::time::Instant::now() < fin
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        store::save(QUEUE, &self.entries);
        store::save(SETTINGS, &self.settings);
    }
}

/// Efface le `.part` et l'état d'un téléchargement annulé.
fn delete_parts(e: &Entry) {
    if let Some(n) = &e.name {
        let cible = Path::new(&e.dir).join(n);
        let _ = std::fs::remove_file(engine::part_path(&cible));
        let _ = std::fs::remove_file(engine::state_path(&cible));
    }
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
