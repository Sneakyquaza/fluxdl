//! Fenêtre egui : file de téléchargements, détail par connexion, comparatif, réglages.

mod draw;

use crate::i18n::t;
use crate::store::{self, Entry, Lang, Settings, Status};
use dlcore::engine::{self, Control, Live, Options, Outcome, Segment, WorkerStat};
use dlcore::APP_NAME;
use draw::*;
use eframe::egui::{self, Margin, RichText, Ui};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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
}

impl App {
    pub fn new(ctx: &egui::Context, capture: Option<PathBuf>) -> Self {
        installer_style(ctx);
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

    fn header(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(APP_NAME.to_uppercase())
                    .font(titre(24.0))
                    .color(CYAN),
            );
            ui.label(
                RichText::new(t(self.settings.lang, "Done early? Take half."))
                    .font(titre(11.0))
                    .color(AMBRE),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for (tab, nom) in [
                    (Tab::Settings, "Settings"),
                    (Tab::Benchmark, "Benchmark"),
                    (Tab::Downloads, "Downloads"),
                ] {
                    let actif = self.tab == tab;
                    let b = egui::Button::new(
                        RichText::new(t(self.settings.lang, nom))
                            .font(titre(13.0))
                            .color(if actif { FOND } else { TEXTE }),
                    )
                    .fill(if actif { CYAN } else { PANNEAU });
                    if ui.add(b).clicked() {
                        self.tab = tab;
                    }
                }
            });
        });
    }

    fn downloads(&mut self, ui: &mut Ui) {
        carte(LIGNE).show(ui, |ui| {
            ui.horizontal(|ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.url)
                        .hint_text(t(self.settings.lang, "Paste a link (http/https)…"))
                        .desired_width(ui.available_width() - 120.0),
                );
                let entree = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let b = egui::Button::new(
                    RichText::new(t(self.settings.lang, "Download"))
                        .font(titre(13.0))
                        .color(FOND),
                )
                .fill(CYAN);
                if ui.add_sized([110.0, 28.0], b).clicked() || entree {
                    self.add();
                }
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new(t(self.settings.lang, "Folder")).color(DISCRET));
                ui.add(egui::TextEdit::singleline(&mut self.settings.folder).desired_width(300.0));
                ui.label(RichText::new("sha256").color(DISCRET));
                ui.add(
                    egui::TextEdit::singleline(&mut self.sha)
                        .hint_text(t(self.settings.lang, "optional"))
                        .desired_width(ui.available_width() - 150.0),
                );
                ui.label(RichText::new(t(self.settings.lang, "Connections")).color(DISCRET));
                ui.add(
                    egui::DragValue::new(&mut self.settings.connections)
                        .range(1..=engine::MAX_CONNECTIONS),
                );
            });
        });
        ui.add_space(4.0);
        // Détail à droite (une barre par connexion), file à gauche.
        if let Some(id) = self.selected {
            egui::Panel::right("detail")
                .exact_size((ui.available_width() * 0.5).max(360.0))
                .resizable(false)
                .show_separator_line(false)
                .frame(egui::Frame::new().inner_margin(Margin {
                    left: 10,
                    ..Default::default()
                }))
                .show(ui, |ui| self.detail(ui, id));
        }
        let mut action = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.entries.is_empty() {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new(t(self.settings.lang, "No downloads yet"))
                                .font(titre(14.0))
                                .color(DISCRET),
                        );
                    });
                }
                for (i, e) in self.entries.iter().enumerate() {
                    if let Some(a) = self.entry_card(ui, e) {
                        action = Some((i, a));
                    }
                }
            });
        if let Some((i, a)) = action {
            self.act(i, a);
        }
    }

    fn entry_card(&self, ui: &mut Ui, e: &Entry) -> Option<Act> {
        let job = self.jobs.get(&e.id);
        let selectionne = self.selected == Some(e.id);
        let (etat, c) = self.status_label(&e.status);
        let mut action = None;
        let r = carte(if selectionne { CYAN } else { LIGNE }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let nom = e.name.clone().unwrap_or_else(|| e.url.clone());
                ui.label(RichText::new(etat).font(titre(10.0)).color(c));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let bouton = |ui: &mut Ui, s: &str| {
                        ui.button(RichText::new(s).font(titre(10.0))).clicked()
                    };
                    match e.status {
                        Status::Running | Status::Queued => {
                            if bouton(ui, t(self.settings.lang, "Cancel")) {
                                action = Some(Act::Cancel);
                            }
                            if bouton(ui, t(self.settings.lang, "Pause")) {
                                action = Some(Act::Pause);
                            }
                        }
                        Status::Paused | Status::Failed(_) => {
                            if bouton(ui, t(self.settings.lang, "Cancel")) {
                                action = Some(Act::Cancel);
                            }
                            if bouton(ui, t(self.settings.lang, "Resume")) {
                                action = Some(Act::Resume);
                            }
                        }
                        Status::Done | Status::Canceled => {
                            if bouton(ui, "×") {
                                action = Some(Act::Remove);
                            }
                        }
                    }
                    if bouton(ui, t(self.settings.lang, "Folder")) {
                        open_folder(Path::new(&e.dir));
                    }
                    // Le nom prend la place qui reste, tronqué.
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(RichText::new(nom).font(titre(13.0))).truncate());
                    });
                });
            });
            let frac = match (e.size, e.status == Status::Done) {
                (_, true) => 1.0,
                (Some(s), _) if s > 0 => e.done as f32 / s as f32,
                _ => 0.0,
            };
            mini(ui, frac, c);
            let mut infos = vec![match e.size {
                Some(s) => format!("{} / {}", octets(e.done.min(s)), octets(s)),
                None => octets(e.done),
            }];
            infos.push(format!("{:.0} %", frac * 100.0));
            if let Some(j) = job.filter(|_| e.status == Status::Running) {
                infos.push(format!("{}/s", octets(j.speed as u64)));
                if let (Some(s), true) = (e.size, j.speed > 1.0) {
                    infos.push(format!(
                        "ETA {}",
                        duree((s - e.done.min(s)) as f64 / j.speed)
                    ));
                }
                infos.push(format!(
                    "{} × {}",
                    e.connections,
                    t(self.settings.lang, "connections")
                ));
            }
            if let Status::Failed(m) = &e.status {
                infos.push(m.clone());
            }
            ui.label(RichText::new(infos.join("  ·  ")).color(DISCRET));
        });
        if r.response.interact(egui::Sense::click()).clicked() && action.is_none() {
            action = Some(Act::Select);
        }
        action
    }

    fn status_label(&self, s: &Status) -> (&'static str, egui::Color32) {
        match s {
            Status::Queued => (t(self.settings.lang, "QUEUED"), DISCRET),
            Status::Running => (t(self.settings.lang, "LIVE"), CYAN),
            Status::Paused => (t(self.settings.lang, "PAUSED"), AMBRE),
            Status::Done => (t(self.settings.lang, "DONE"), VERT),
            Status::Failed(_) => (t(self.settings.lang, "ERROR"), ALERTE),
            Status::Canceled => (t(self.settings.lang, "CANCELED"), DISCRET),
        }
    }

    fn act(&mut self, i: usize, a: Act) {
        let id = self.entries[i].id;
        match a {
            Act::Select => {
                self.selected = if self.selected == Some(id) {
                    None
                } else {
                    Some(id)
                };
            }
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
                self.jobs.remove(&id);
                if self.selected == Some(id) {
                    self.selected = None;
                }
            }
        }
    }

    fn detail(&self, ui: &mut Ui, id: u64) {
        let Some(e) = self.entries.iter().find(|e| e.id == id) else {
            return;
        };
        carte(LIGNE).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(ui.available_height());
            let Some(j) = self.jobs.get(&id) else {
                ui.label(
                    RichText::new(t(
                        self.settings.lang,
                        "No live data: resume to see the connections.",
                    ))
                    .color(DISCRET),
                );
                return;
            };
            let l = j.ctl.snapshot();
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(t(self.settings.lang, "CONNECTIONS"))
                        .font(titre(13.0))
                        .color(CYAN),
                );
                ui.label(
                    RichText::new(format!(
                        "{} {}  ·  {} {}",
                        l.workers.len(),
                        t(self.settings.lang, "active"),
                        l.steals,
                        t(self.settings.lang, "steals (done early? take half)")
                    ))
                    .color(DISCRET),
                );
                if l.restarted {
                    ui.label(
                        RichText::new(t(self.settings.lang, "remote file changed: restarted"))
                            .color(AMBRE),
                    );
                }
                if !l.ranges && e.status == Status::Running {
                    ui.label(
                        RichText::new(t(self.settings.lang, "server refuses ranges: 1 connection"))
                            .color(AMBRE),
                    );
                }
            });
            ui.label(
                RichText::new(t(self.settings.lang, "File map"))
                    .font(titre(10.0))
                    .color(DISCRET),
            );
            carte_fichier(ui, &l, 22.0);
            ui.add_space(4.0);
            let reste = (ui.available_height() - 90.0).max(60.0);
            egui::ScrollArea::vertical()
                .id_salt("connexions")
                .max_height(reste)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    connexions(ui, &l, &j.speeds, t(self.settings.lang, "STOLEN"))
                });
            ui.label(
                RichText::new(t(self.settings.lang, "Total speed"))
                    .font(titre(10.0))
                    .color(DISCRET),
            );
            let h: Vec<f32> = j.history.iter().copied().collect();
            courbe(ui, &h, (ui.available_height() - 16.0).clamp(40.0, 160.0));
        });
    }

    fn benchmark(&mut self, ui: &mut Ui) {
        carte(LIGNE).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(t(self.settings.lang, "BENCHMARK")).font(titre(16.0)).color(CYAN));
            ui.label(
                RichText::new(t(
                    self.settings.lang,
                    "Downloads the same link with 1 connection, then with N, to a temporary file deleted afterwards.",
                ))
                .color(DISCRET),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.bench.url)
                    .hint_text(t(self.settings.lang, "Paste a link (http/https)…"))
                    .desired_width(f32::INFINITY),
            );
            ui.horizontal(|ui| {
                ui.label(RichText::new(t(self.settings.lang, "Connections")).color(DISCRET));
                ui.add(egui::Slider::new(&mut self.bench.connections, 2..=engine::MAX_CONNECTIONS));
                ui.label(RichText::new(t(self.settings.lang, "Max seconds per run")).color(DISCRET));
                ui.add(egui::DragValue::new(&mut self.bench.secs).range(2..=60));
            });
            let en_cours = self.bench.handle.is_some();
            ui.horizontal(|ui| {
                if !en_cours {
                    let b = egui::Button::new(RichText::new(t(self.settings.lang, "Run benchmark")).font(titre(13.0)).color(FOND))
                        .fill(CYAN);
                    if ui.add(b).clicked() && self.bench.url.trim().starts_with("http") {
                        self.start_bench();
                    }
                } else if ui.button(RichText::new(t(self.settings.lang, "Stop")).font(titre(13.0))).clicked() {
                    self.bench.cancel.store(true, Ordering::Relaxed);
                    if let Some((_, c)) = &*self.bench.live.lock().unwrap_or_else(|e| e.into_inner()) {
                        c.request_stop();
                    }
                }
            });
        });
        ui.add_space(6.0);
        let live = self
            .bench
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let (true, Some((phase, c))) = (self.bench.handle.is_some(), live) {
            carte(LIGNE).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let n = if phase == 1 {
                    1
                } else {
                    self.bench.connections
                };
                ui.label(
                    RichText::new(format!(
                        "{} {n} {}…",
                        t(self.settings.lang, "Measuring"),
                        t(self.settings.lang, "connections")
                    ))
                    .font(titre(12.0))
                    .color(AMBRE),
                );
                let l = c.snapshot();
                carte_fichier(ui, &l, 18.0);
                connexions(ui, &l, &[], t(self.settings.lang, "STOLEN"));
            });
        }
        if let Some(r) = &self.bench.result {
            carte(LIGNE).show(ui, |ui| {
                ui.set_width(ui.available_width());
                match r {
                    Ok((v1, vn)) => {
                        let max = v1.max(*vn).max(1.0);
                        for (n, v, c) in [(1, *v1, DISCRET), (self.bench.connections, *vn, ROSE)] {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "{n} {}",
                                        t(self.settings.lang, if n == 1 { "CONNECTION" } else { "CONNECTIONS" })
                                    ))
                                        .font(titre(11.0)),
                                );
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(RichText::new(format!("{}/s", octets(v as u64))).font(titre(12.0)));
                                });
                            });
                            barre_comparatif(ui, (v / max) as f32, c);
                        }
                        ui.add_space(6.0);
                        let x = vn / v1.max(1.0);
                        ui.label(RichText::new(format!("×{x:.1}")).font(titre(34.0)).color(if x >= 1.5 { VERT } else { AMBRE }));
                        if x < 1.5 {
                            ui.label(
                                RichText::new(t(
                    self.settings.lang,
                                    "Little gain: the server or your line is probably already saturated by one connection.",
                                ))
                                .color(DISCRET),
                            );
                        }
                    }
                    Err(e) => {
                        ui.label(RichText::new(e).color(ALERTE));
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
        carte(LIGNE).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(t(self.settings.lang, "SETTINGS")).font(titre(16.0)).color(CYAN));
            egui::Grid::new("reglages").num_columns(2).spacing([24.0, 14.0]).show(ui, |ui| {
                ui.label(t(self.settings.lang, "Connections per download"));
                ui.add(egui::Slider::new(&mut self.settings.connections, 1..=engine::MAX_CONNECTIONS));
                ui.end_row();
                ui.label(t(self.settings.lang, "Default folder"));
                ui.add(egui::TextEdit::singleline(&mut self.settings.folder).desired_width(420.0));
                ui.end_row();
                ui.label(t(self.settings.lang, "Speed limit (MB/s, 0 = none)"));
                ui.add(egui::DragValue::new(&mut self.settings.speed_limit_mb).range(0.0..=10_000.0).speed(0.5));
                ui.end_row();
                ui.label(t(self.settings.lang, "Language"));
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.settings.lang, Lang::En, "English");
                    ui.selectable_value(&mut self.settings.lang, Lang::Fr, "Français");
                });
                ui.end_row();
            });
            ui.add_space(8.0);
            ui.label(
                RichText::new(t(
                    self.settings.lang,
                    "Up to 32 connections per file. Unfinished downloads resume where they stopped, even after a crash.",
                ))
                .color(DISCRET),
            );
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
        let vues: [(u32, Tab, Lang, &str); 4] = [
            (10, Tab::Downloads, Lang::En, "downloads"),
            (25, Tab::Benchmark, Lang::En, "benchmark"),
            (40, Tab::Settings, Lang::En, "settings"),
            (55, Tab::Downloads, Lang::Fr, "downloads_fr"),
        ];
        for (k, tab, lang, nom) in vues {
            if f == k - 6 {
                self.tab = tab;
                self.settings.lang = lang;
            }
            if f == k {
                ctx.send_viewport_cmd(screenshot(dir.join(format!("{nom}.png"))));
            }
        }
        if f > 70 {
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
        if self.bench.handle.as_ref().is_some_and(|h| h.is_finished()) {
            let h = self.bench.handle.take().map(|h| h.join());
            self.bench.result = h.map(|r| r.unwrap_or_else(|_| Err("panic".into())));
        }
        self.capture_step(&ctx);
        egui::Panel::top("entete")
            .frame(
                egui::Frame::new()
                    .fill(PANNEAU)
                    .inner_margin(Margin::symmetric(18, 10)),
            )
            .show(ui, |ui| self.header(ui));
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(FOND)
                    .inner_margin(Margin::symmetric(16, 12)),
            )
            .show(ui, |ui| match self.tab {
                Tab::Downloads => self.downloads(ui),
                Tab::Benchmark => self.benchmark(ui),
                Tab::Settings => self.settings_tab(ui),
            });
        self.persist(now);
        ctx.request_repaint_after(Duration::from_millis(100));
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
    let n = n as f64;
    match n {
        n if n >= 1e9 => format!("{:.2} GB", n / 1e9),
        n if n >= 1e6 => format!("{:.1} MB", n / 1e6),
        n if n >= 1e3 => format!("{:.0} kB", n / 1e3),
        n => format!("{n:.0} B"),
    }
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
