//! Fenêtres par-dessus l'appli (choix du format d'une vidéo, fichiers d'un torrent, outils
//! vidéo, confirmations), bulle de mise à jour, cartes de réglages de la v0.2.

use super::*;
use crate::i18n::tr;

/// Largeur des fenêtres modales.
const LARGEUR: f32 = 500.0;

impl App {
    /// Résultats arrivés des fils (formats, aperçu, outils) et suites des fenêtres.
    pub(super) fn suivre_modale(&mut self) {
        match &mut self.modale {
            Some(Modale::Video {
                rx, infos, choix, ..
            }) => {
                if let Some(r) = rx.as_ref().and_then(|r| r.try_recv().ok()) {
                    if let Ok(i) = &r {
                        // Premier choix disponible par défaut.
                        if let Some(c) = i.choix.iter().find(|c| c.1) {
                            *choix = c.0;
                        }
                    }
                    *infos = Some(r);
                    *rx = None;
                }
            }
            Some(Modale::Torrent {
                rx, apercu, coches, ..
            }) => {
                if let Some(r) = rx.as_ref().and_then(|r| r.try_recv().ok()) {
                    if let Ok(a) = &r {
                        *coches = vec![true; a.fichiers.len()];
                    }
                    *apercu = Some(r);
                    *rx = None;
                }
            }
            Some(Modale::Outils { url, etat }) => {
                let fini = match etat {
                    EtatOutils::Lecture(rx) => {
                        if let Ok(r) = rx.try_recv() {
                            *etat = match r {
                                Ok(p) if p.ytdlp.is_none() && p.ffmpeg.is_none() => {
                                    EtatOutils::Erreur("Nothing to download.")
                                }
                                Ok(p) => EtatOutils::Pret(p),
                                Err(m) => EtatOutils::Erreur(m),
                            };
                        }
                        None
                    }
                    EtatOutils::Installation(_, rx, _) => match rx.try_recv() {
                        Ok(Ok(())) => Some(std::mem::take(url)),
                        Ok(Err(m)) => {
                            *etat = EtatOutils::Erreur(m);
                            None
                        }
                        Err(_) => None,
                    },
                    _ => None,
                };
                if let Some(u) = fini {
                    self.outils = video::trouver();
                    self.settings.ytdlp_updated = secondes();
                    self.modale = None;
                    if !u.is_empty() {
                        self.ouvrir_video(u);
                    }
                }
            }
            _ => {}
        }
    }

    /// Voile sur toute la fenêtre (bloque les clics dessous) et fenêtre de verre centrée.
    pub(super) fn modale(&mut self, ctx: &egui::Context) {
        if self.modale.is_none() {
            return;
        }
        let ecran = ctx.content_rect();
        egui::Area::new(egui::Id::unique("voile"))
            .order(egui::Order::Middle)
            .fixed_pos(ecran.min)
            .show(ctx, |ui| {
                let (r, _) = ui.allocate_exact_size(ecran.size(), Sense::click());
                ui.painter().rect_filled(r, 0.0, nuit(70));
            });
        let mut fermer = false;
        egui::Area::new(egui::Id::unique("modale"))
            .order(egui::Order::Foreground)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                let fond = ui.painter().add(egui::Shape::Noop);
                let rep = egui::Frame::NONE
                    .inner_margin(Margin::same(24))
                    .show(ui, |ui| {
                        ui.set_width(LARGEUR);
                        fermer = self.contenu_modale(ui);
                    });
                let r = rep.response.rect;
                ui.painter().set(fond, verre(ui.ctx(), r, 16, 0.86));
            });
        if fermer || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if let Some(Modale::Outils {
                etat: EtatOutils::Installation(_, _, stop),
                ..
            }) = &self.modale
            {
                stop.store(true, Ordering::Relaxed);
            }
            self.modale = None;
        }
    }

    /// Contenu de la fenêtre ouverte ; vrai = fermer.
    fn contenu_modale(&mut self, ui: &mut Ui) -> bool {
        let l = self.settings.lang;
        let Some(m) = self.modale.take() else {
            return true;
        };
        let mut garder = true;
        let mut fermer = false;
        match m {
            Modale::MajActifs(n) => {
                titre_modale(ui, Icone::Actualiser, t(l, "Update now?"));
                paragraphe(
                    ui,
                    &format!(
                        "{} {n}. {}",
                        t(l, "Active downloads:"),
                        t(
                            l,
                            "They will pause, fluxdl restarts with the new version, and they resume on their own."
                        )
                    ),
                );
                match boutons(ui, t(l, "Cancel"), t(l, "Update and restart"), true) {
                    Some(true) => {
                        self.maj_auto = true;
                        self.maj.telecharger();
                        fermer = true;
                    }
                    Some(false) => fermer = true,
                    None => {}
                }
                if !fermer {
                    self.modale = Some(Modale::MajActifs(n));
                }
                return fermer;
            }
            Modale::Association => {
                titre_modale(ui, Icone::Aimant, t(l, "Open torrents with fluxdl?"));
                paragraphe(
                    ui,
                    t(
                        l,
                        "Magnet links in your browser and .torrent files in Explorer will open here. You can change this later in Settings.",
                    ),
                );
                match boutons(ui, t(l, "Not now"), t(l, "Yes"), true) {
                    Some(oui) => {
                        self.settings.handler_asked = true;
                        if oui {
                            self.settings.torrent_handler = torrent::associer(true);
                        }
                        return true;
                    }
                    None => self.modale = Some(Modale::Association),
                }
                return false;
            }
            Modale::Video {
                url,
                rx,
                infos,
                mut choix,
            } => {
                titre_modale(ui, Icone::Lecture, t(l, "Download a video"));
                match &infos {
                    None => attente(ui, t(l, "Reading the available formats…")),
                    Some(Err(m)) => {
                        erreur(ui, tr(l, m));
                        fermer = boutons(ui, t(l, "Close"), "", false).is_some();
                    }
                    Some(Ok(i)) => {
                        let g = ligne(ui, i.titre.as_str(), demi(14.5), TEXTE, LARGEUR);
                        let (r, _) = ui.allocate_exact_size(g.size(), Sense::hover());
                        ui.painter().galley(r.min, g, TEXTE);
                        if let Some(d) = i.duree {
                            ui.label(
                                egui::RichText::new(duree(d))
                                    .font(regulier(12.5))
                                    .color(TEXTE3),
                            );
                        }
                        ui.add_space(10.0);
                        for &(q, dispo, taille) in &i.choix {
                            let nom = match q {
                                video::Qualite::Best => format!(
                                    "{} ({} {})",
                                    t(l, "Best quality"),
                                    t(l, "up to"),
                                    video::nom_hauteur(i.hauteur_max)
                                ),
                                video::Qualite::P1080 => "1080p".into(),
                                video::Qualite::P720 => "720p".into(),
                                video::Qualite::M4a => format!("{} (M4A)", t(l, "Audio only")),
                                video::Qualite::Mp3 => format!("{} (MP3)", t(l, "Audio only")),
                            };
                            let droite = taille.map_or("—".into(), |t| format!("~{}", octets(t)));
                            if choix_radio(ui, choix == q, dispo, &nom, &droite) {
                                choix = q;
                            }
                        }
                        ui.add_space(6.0);
                        let aide = t(
                            l,
                            "Downloaded by yt-dlp; picture and sound are merged by ffmpeg.",
                        );
                        ui.label(egui::RichText::new(aide).font(regulier(12.0)).color(TEXTE3));
                        match boutons(ui, t(l, "Cancel"), t(l, "Download"), true) {
                            Some(true) => {
                                let estimate =
                                    i.choix.iter().find(|c| c.0 == choix).and_then(|c| c.2);
                                let titre = i.titre.clone();
                                let e = self.nouvelle(
                                    url.clone(),
                                    Kind::Video {
                                        quality: choix,
                                        estimate,
                                    },
                                );
                                e.name = Some(titre);
                                e.size = estimate;
                                garder = false;
                            }
                            Some(false) => fermer = true,
                            None => {}
                        }
                    }
                }
                if infos.is_none() {
                    fermer = boutons(ui, t(l, "Cancel"), "", false).is_some();
                }
                if garder && !fermer {
                    self.modale = Some(Modale::Video {
                        url,
                        rx,
                        infos,
                        choix,
                    });
                }
            }
            Modale::Torrent {
                source,
                rx,
                apercu,
                mut coches,
            } => {
                titre_modale(ui, Icone::Aimant, t(l, "Add a torrent"));
                match &apercu {
                    None => {
                        let m = if source.starts_with("magnet:") {
                            t(l, "Asking peers for the torrent's file list…")
                        } else {
                            t(l, "Reading the torrent…")
                        };
                        attente(ui, m);
                        fermer = boutons(ui, t(l, "Cancel"), "", false).is_some();
                    }
                    Some(Err(m)) => {
                        erreur(ui, tr(l, m));
                        fermer = boutons(ui, t(l, "Close"), "", false).is_some();
                    }
                    Some(Ok(a)) => {
                        let total: u64 = a
                            .fichiers
                            .iter()
                            .zip(&coches)
                            .filter(|(_, c)| **c)
                            .map(|(f, _)| f.taille)
                            .sum();
                        let g = ligne(ui, a.nom.as_str(), demi(14.5), TEXTE, LARGEUR);
                        let (r, _) = ui.allocate_exact_size(g.size(), Sense::hover());
                        ui.painter().galley(r.min, g, TEXTE);
                        let n = coches.iter().filter(|c| **c).count();
                        ui.label(
                            egui::RichText::new(format!(
                                "{n} / {} {}, {}",
                                a.fichiers.len(),
                                compte(l, a.fichiers.len(), "files"),
                                octets(total)
                            ))
                            .font(regulier(12.5))
                            .color(TEXTE3),
                        );
                        ui.add_space(8.0);
                        egui::ScrollArea::vertical()
                            .max_height(240.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                for (f, c) in a.fichiers.iter().zip(coches.iter_mut()) {
                                    coche(ui, c, &f.nom, &octets(f.taille));
                                }
                            });
                        match boutons(ui, t(l, "Cancel"), t(l, "Download"), n > 0) {
                            Some(true) => {
                                self.ajouter_torrent(&source, a, &coches);
                                garder = false;
                            }
                            Some(false) => fermer = true,
                            None => {}
                        }
                    }
                }
                if garder && !fermer {
                    self.modale = Some(Modale::Torrent {
                        source,
                        rx,
                        apercu,
                        coches,
                    });
                }
            }
            Modale::Outils { url, etat } => {
                titre_modale(ui, Icone::Lecture, t(l, "Video tools needed"));
                match &etat {
                    EtatOutils::Lecture(_) => {
                        attente(ui, t(l, "Looking up the latest yt-dlp and ffmpeg…"));
                        fermer = boutons(ui, t(l, "Cancel"), "", false).is_some();
                    }
                    EtatOutils::Pret(p) => {
                        paragraphe(
                            ui,
                            t(
                                l,
                                "Videos need yt-dlp (reads and downloads the video) and ffmpeg (merges picture and sound). fluxdl can download the official builds from GitHub and verify their checksums:",
                            ),
                        );
                        ui.add_space(8.0);
                        let lignes =
                            p.ytdlp
                                .iter()
                                .map(|f| ("yt-dlp", "github.com/yt-dlp/yt-dlp", f.0.size))
                                .chain(p.ffmpeg.iter().map(|f| {
                                    ("ffmpeg", "github.com/yt-dlp/FFmpeg-Builds", f.0.size)
                                }));
                        for (nom, depot, taille) in lignes {
                            let (r, _) =
                                ui.allocate_exact_size(vec2(LARGEUR, 40.0), Sense::hover());
                            let pt = ui.painter();
                            pt.add(vdeg(r, 8, blanc(200), blanc(130)));
                            pt.rect_stroke(r, 8.0, Stroke::new(1.0, BORD), StrokeKind::Inside);
                            tuile_icone(
                                pt,
                                Rect::from_min_size(r.min + vec2(10.0, 8.0), Vec2::splat(24.0)),
                                Icone::Telecharger,
                                GEL_BLEU,
                            );
                            texte(
                                ui,
                                pos2(r.left() + 44.0, r.top() + 13.0),
                                Align2::LEFT_CENTER,
                                nom,
                                demi(13.5),
                                TEXTE,
                            );
                            texte(
                                ui,
                                pos2(r.left() + 44.0, r.top() + 28.0),
                                Align2::LEFT_CENTER,
                                depot,
                                regulier(12.0),
                                TEXTE3,
                            );
                            texte(
                                ui,
                                r.right_center() - vec2(12.0, 0.0),
                                Align2::RIGHT_CENTER,
                                &octets(taille),
                                demi(13.5),
                                TEXTE,
                            );
                            ui.add_space(4.0);
                        }
                        let ok = format!("{} ({})", t(l, "Download"), octets(p.taille()));
                        match boutons(ui, t(l, "Cancel"), &ok, true) {
                            Some(true) => {
                                let prog = Arc::new(Mutex::new((0, p.taille())));
                                let stop = Arc::new(AtomicBool::new(false));
                                let (tx, rx) = std::sync::mpsc::channel();
                                let (pa, pr, st) = (p.clone(), prog.clone(), stop.clone());
                                let c = ui.ctx().clone();
                                std::thread::spawn(move || {
                                    let mut dernier = 0;
                                    let r = video::installer(&pa, &st, |n, tot| {
                                        *pr.lock().unwrap_or_else(|e| e.into_inner()) = (n, tot);
                                        if n - dernier > tot / 200 {
                                            dernier = n;
                                            c.request_repaint();
                                        }
                                    });
                                    let _ = tx.send(r);
                                    c.request_repaint();
                                });
                                self.modale = Some(Modale::Outils {
                                    url,
                                    etat: EtatOutils::Installation(prog, rx, stop),
                                });
                                return false;
                            }
                            Some(false) => fermer = true,
                            None => {}
                        }
                    }
                    EtatOutils::Installation(p, _, _) => {
                        let (n, tot) = *p.lock().unwrap_or_else(|e| e.into_inner());
                        attente(ui, t(l, "Downloading and checking…"));
                        let (r, _) = ui.allocate_exact_size(vec2(LARGEUR, 14.0), Sense::hover());
                        jauge(
                            ui.painter(),
                            r,
                            n as f32 / tot.max(1) as f32,
                            rgb(0x3c, 0xbf, 0x2a),
                            true,
                        );
                        ui.label(
                            egui::RichText::new(format!("{} / {}", octets(n), octets(tot)))
                                .font(regulier(12.5))
                                .color(TEXTE3),
                        );
                        ui.ctx().request_repaint_after(Duration::from_millis(250));
                        fermer = boutons(ui, t(l, "Cancel"), "", false).is_some();
                    }
                    EtatOutils::Erreur(m) => {
                        erreur(ui, t(l, m));
                        fermer = boutons(ui, t(l, "Close"), "", false).is_some();
                    }
                }
                if !fermer {
                    self.modale = Some(Modale::Outils { url, etat });
                }
            }
        }
        fermer
    }

    /// Torrent choisi : `.torrent` gardé pour la reprise, entrée ajoutée (déjà là : choisie).
    fn ajouter_torrent(&mut self, source: &str, a: &torrent::Apercu, coches: &[bool]) {
        if let Some(e) = self
            .entries
            .iter()
            .find(|e| matches!(&e.kind, Kind::Torrent { hash, .. } if *hash == a.hash))
        {
            self.selected = Some(e.id);
            return;
        }
        let _ = std::fs::create_dir_all(torrent::dossier());
        if let Err(e) = engine::write_atomic(&torrent::garde(&a.hash), &a.octets) {
            self.modale = Some(Modale::Torrent {
                source: source.into(),
                rx: None,
                apercu: Some(Err(e.to_string())),
                coches: Vec::new(),
            });
            return;
        }
        let files = (!coches.iter().all(|c| *c))
            .then(|| (0..coches.len()).filter(|&i| coches[i]).collect());
        let taille = a
            .fichiers
            .iter()
            .zip(coches)
            .filter(|(_, c)| **c)
            .map(|(f, _)| f.taille)
            .sum();
        let e = self.nouvelle(
            source.into(),
            Kind::Torrent {
                hash: a.hash.clone(),
                files,
                started: false,
            },
        );
        e.name = Some(a.nom.clone());
        e.size = Some(taille);
        if cfg!(windows) && !self.settings.handler_asked {
            self.modale = Some(Modale::Association);
        }
    }

    /// Bulle de notification (en bas à droite) : nouvelle version, téléchargement, échec.
    pub(super) fn bulle_maj(&mut self, ctx: &egui::Context) {
        let l = self.settings.lang;
        let etat = self.maj.etat();
        let (titre, sous) = match &etat {
            maj::Etat::Dispo(a) if self.maj_cachee.as_ref() != Some(&a.version) => (
                format!(
                    "{} {} {}",
                    t(l, "New version"),
                    a.version,
                    t(l, "available")
                ),
                format!("{} {}.", t(l, "You have"), maj::VERSION),
            ),
            maj::Etat::Telechargement(v, n, tot) => (
                format!("{} {v}…", t(l, "Downloading")),
                format!("{} / {}", octets(*n), octets(*tot)),
            ),
            maj::Etat::Pret(v, _) => (
                format!("{} {v} {}", t(l, "Version"), t(l, "is ready")),
                t(l, "Restarting to install it…").into(),
            ),
            maj::Etat::Echec(m) => (t(l, "Update failed").into(), t(l, m).into()),
            _ => return,
        };
        let ecran = ctx.content_rect();
        let taille = vec2(380.0, 112.0);
        let pos = ecran.max - taille - vec2(28.0, 26.0);
        egui::Area::new(egui::Id::unique("bulle-maj"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                let (r, _) = ui.allocate_exact_size(taille, Sense::hover());
                let p = ui.painter();
                p.add(lueur(
                    r.expand(2.0),
                    rgb(0x5c, 0xc8, 0xff).gamma_multiply(0.35),
                    14.0,
                ));
                p.add(verre(ui.ctx(), r, 16, 0.84));
                let c = pos2(r.left() + 34.0, r.top() + 36.0);
                orbe(p, c, 17.0, GEL_VERT);
                icone_relief(
                    p,
                    Rect::from_center_size(c, Vec2::splat(16.0)),
                    Icone::Actualiser,
                );
                let x = r.left() + 62.0;
                let g = ligne(ui, titre, demi(14.5), TITRE, r.right() - x - 14.0);
                p.galley(pos2(x, r.top() + 27.0 - g.size().y / 2.0), g, TITRE);
                let g = ligne(ui, sous, regulier(12.5), TEXTE2, r.right() - x - 14.0);
                p.galley(pos2(x, r.top() + 47.0 - g.size().y / 2.0), g, TEXTE2);
                let bas = Rect::from_min_max(
                    pos2(x, r.bottom() - 44.0),
                    pos2(r.right() - 16.0, r.bottom() - 14.0),
                );
                match &etat {
                    maj::Etat::Dispo(a) => {
                        let bp = Rect::from_min_size(
                            pos2(bas.right() - 130.0, bas.top()),
                            vec2(130.0, 30.0),
                        );
                        let bl = Rect::from_min_size(
                            pos2(bp.left() - 98.0, bas.top()),
                            vec2(90.0, 30.0),
                        );
                        let plus_tard = a_la_place(ui, bl, |ui| {
                            bouton_secondaire(ui, t(l, "Later"), None, bl.size())
                        });
                        let maj = a_la_place(ui, bp, |ui| {
                            bouton_gel(
                                ui,
                                t(l, "Update"),
                                Some(Icone::Telecharger),
                                bp.size(),
                                GEL_VERT,
                            )
                        });
                        if plus_tard.clicked() {
                            self.maj_cachee = Some(a.version.clone());
                        }
                        if maj.clicked() {
                            self.demander_maj();
                        }
                    }
                    maj::Etat::Telechargement(_, n, tot) => {
                        let j = Rect::from_min_size(
                            pos2(x, bas.center().y - 6.0),
                            vec2(bas.width(), 12.0),
                        );
                        jauge(
                            p,
                            j,
                            *n as f32 / (*tot).max(1) as f32,
                            rgb(0x3c, 0xbf, 0x2a),
                            true,
                        );
                    }
                    maj::Etat::Echec(_) => {
                        let b = Rect::from_min_size(
                            pos2(bas.right() - 90.0, bas.top()),
                            vec2(90.0, 30.0),
                        );
                        if a_la_place(ui, b, |ui| {
                            bouton_secondaire(ui, t(l, "Close"), None, b.size())
                        })
                        .clicked()
                        {
                            self.maj.mettre(maj::Etat::Rien);
                        }
                    }
                    _ => {}
                }
            });
    }

    /// « Mettre à jour » : Linux → page des releases ; Windows → accord si des
    /// téléchargements tournent, puis téléchargement, installation et redémarrage.
    pub(super) fn demander_maj(&mut self) {
        if !maj::AUTO {
            open_folder(Path::new(maj::PAGE_RELEASES));
            return;
        }
        let n = self
            .entries
            .iter()
            .filter(|e| e.status == Status::Running && !self.partage(e.id))
            .count();
        if n > 0 {
            self.modale = Some(Modale::MajActifs(n));
        } else {
            self.maj_auto = true;
            self.maj.telecharger();
        }
    }

    /// Cartes des réglages ajoutées en v0.2 : extension, torrents, vidéos, mises à jour.
    pub(super) fn reglages_v2(&mut self, ui: &mut Ui) {
        let l = self.settings.lang;
        carte(ui, Margin::symmetric(20, 6), |ui| {
            ui.set_width(ui.available_width());
            titre_section(ui, t(l, "Browser extension"));
            reglage(
                ui,
                t(l, "Accept links from the browser"),
                t(
                    l,
                    "Local server on 127.0.0.1 only, for the fluxdl extension.",
                ),
                |ui| {
                    if interrupteur(ui, &mut self.settings.server).changed() {
                        self.sync_jeton();
                    }
                },
            );
            separateur(ui);
            reglage(
                ui,
                t(l, "Pairing token"),
                t(l, "Paste it in the extension's options to pair it."),
                |ui| {
                    let c = ui.ctx().clone();
                    if bouton_principal(ui, t(l, "Copy"), Some(Icone::Copier), vec2(104.0, 36.0))
                        .clicked()
                    {
                        c.copy_text(self.settings.token.clone());
                    }
                    let r = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover()).0;
                    if bouton_icone_ici(
                        ui,
                        r,
                        egui::Id::unique("jeton-neuf"),
                        Icone::Actualiser,
                        t(l, "New token (unpairs the extension)"),
                    )
                    .clicked()
                    {
                        self.settings.token = serveur::nouveau_jeton();
                        self.sync_jeton();
                    }
                    let fin: String = self
                        .settings
                        .token
                        .chars()
                        .rev()
                        .take(4)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    boite_valeur(ui, &format!("••••••••{fin}"), 120.0);
                },
            );
            separateur(ui);
            reglage(
                ui,
                t(l, "Port"),
                t(
                    l,
                    "Same port in the extension. Takes effect after a restart.",
                ),
                |ui| {
                    ui.add(egui::DragValue::new(&mut self.settings.port).range(1024..=65535));
                },
            );
            if let Some(e) = &self.serveur_err {
                ui.label(
                    egui::RichText::new(format!("{} {e}", t(l, "Server not started:")))
                        .font(regulier(12.5))
                        .color(DANGER),
                );
                ui.add_space(8.0);
            }
        });
        ui.add_space(20.0);
        carte(ui, Margin::symmetric(20, 6), |ui| {
            ui.set_width(ui.available_width());
            titre_section(ui, t(l, "Torrents"));
            reglage(
                ui,
                t(l, "Keep sharing after download"),
                t(l, "Off: a torrent stops as soon as it is complete."),
                |ui| {
                    let mut on = self.settings.seed_ratio > 0.0;
                    if interrupteur(ui, &mut on).changed() {
                        self.settings.seed_ratio = if on { 1.0 } else { 0.0 };
                    }
                    if on {
                        ui.add_space(8.0);
                        let v = &mut self.settings.seed_ratio;
                        boite_valeur(ui, &format!("{} {v:.1}", t(l, "ratio")), 84.0);
                        let mut f = *v as f64;
                        if curseur(ui, &mut f, (0.1, 5.0), false, 0.1, 140.0).changed() {
                            *v = (f * 10.0).round() as f32 / 10.0;
                        }
                    }
                },
            );
            separateur(ui);
            reglage(
                ui,
                t(l, "Open magnet links and .torrent files"),
                if cfg!(windows) {
                    t(l, "Registers fluxdl for your Windows account only.")
                } else {
                    t(l, "Windows only.")
                },
                |ui| {
                    ui.add_enabled_ui(cfg!(windows), |ui| {
                        let mut on = self.settings.torrent_handler;
                        if interrupteur(ui, &mut on).changed() {
                            self.settings.handler_asked = true;
                            self.settings.torrent_handler = torrent::associer(on) && on;
                        }
                    });
                },
            );
        });
        ui.add_space(20.0);
        carte(ui, Margin::symmetric(20, 6), |ui| {
            ui.set_width(ui.available_width());
            titre_section(ui, t(l, "Videos"));
            let o = &self.outils;
            let etat = |p: &Option<PathBuf>| {
                if p.is_some() {
                    t(l, "ready")
                } else {
                    t(l, "missing")
                }
            };
            let aide = format!("yt-dlp : {}, ffmpeg : {}", etat(&o.ytdlp), etat(&o.ffmpeg));
            let complets = o.complets();
            reglage(ui, t(l, "yt-dlp and ffmpeg"), &aide, |ui| {
                if cfg!(windows) && !complets {
                    if bouton_principal(
                        ui,
                        t(l, "Download"),
                        Some(Icone::Telecharger),
                        vec2(132.0, 36.0),
                    )
                    .clicked()
                    {
                        self.ouvrir_video_outils();
                    }
                } else if cfg!(windows)
                    && bouton_secondaire(
                        ui,
                        t(l, "Update yt-dlp"),
                        Some(Icone::Actualiser),
                        vec2(150.0, 36.0),
                    )
                    .clicked()
                {
                    video::mettre_a_jour_ytdlp();
                    self.settings.ytdlp_updated = secondes();
                }
            });
        });
        ui.add_space(20.0);
        carte(ui, Margin::symmetric(20, 6), |ui| {
            ui.set_width(ui.available_width());
            titre_section(ui, t(l, "Updates"));
            let etat = match self.maj.etat() {
                maj::Etat::Rien => String::new(),
                maj::Etat::Recherche => t(l, "Checking…").into(),
                maj::Etat::AJour => t(l, "You have the latest version.").into(),
                maj::Etat::Erreur(m) | maj::Etat::Echec(m) => t(l, m).into(),
                maj::Etat::Dispo(a) => format!(
                    "{} {} {}",
                    t(l, "New version"),
                    a.version,
                    t(l, "available")
                ),
                maj::Etat::Telechargement(v, n, tot) => {
                    format!(
                        "{} {v}… {:.0}%",
                        t(l, "Downloading"),
                        n as f64 * 100.0 / tot.max(1) as f64
                    )
                }
                maj::Etat::Pret(v, _) => format!("{} {v} {}", t(l, "Version"), t(l, "is ready")),
            };
            let aide = format!("{} {}. {etat}", t(l, "Installed version"), maj::VERSION);
            let dispo = matches!(self.maj.etat(), maj::Etat::Dispo(_));
            reglage(ui, t(l, "Check for updates"), &aide, |ui| {
                if dispo {
                    if bouton_gel(
                        ui,
                        t(l, "Update"),
                        Some(Icone::Telecharger),
                        vec2(132.0, 36.0),
                        GEL_VERT,
                    )
                    .clicked()
                    {
                        self.demander_maj();
                    }
                } else if bouton_secondaire(
                    ui,
                    t(l, "Check now"),
                    Some(Icone::Actualiser),
                    vec2(140.0, 36.0),
                )
                .clicked()
                {
                    self.maj.chercher(false);
                }
            });
        });
        ui.add_space(20.0);
    }

    /// Réglages → Vidéos → Télécharger : même fenêtre d'accord, sans vidéo à ouvrir après.
    fn ouvrir_video_outils(&mut self) {
        let o = self.outils.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(video::preparer(&o));
        });
        self.modale = Some(Modale::Outils {
            url: String::new(),
            etat: EtatOutils::Lecture(rx),
        });
    }
}

fn titre_modale(ui: &mut Ui, i: Icone, titre: &str) {
    let (r, _) = ui.allocate_exact_size(vec2(LARGEUR, 34.0), Sense::hover());
    tuile_icone(
        ui.painter(),
        Rect::from_min_size(pos2(r.left(), r.center().y - 15.0), Vec2::splat(30.0)),
        i,
        GEL_BLEU,
    );
    texte(
        ui,
        pos2(r.left() + 42.0, r.center().y),
        Align2::LEFT_CENTER,
        titre,
        demi(18.0),
        TITRE,
    );
    ui.add_space(12.0);
}

fn paragraphe(ui: &mut Ui, s: &str) {
    ui.label(egui::RichText::new(s).font(regulier(13.5)).color(TEXTE2));
}

fn erreur(ui: &mut Ui, s: &str) {
    ui.label(egui::RichText::new(s).font(regulier(13.5)).color(DANGER));
}

fn attente(ui: &mut Ui, s: &str) {
    ui.horizontal(|ui| {
        ui.add(egui::Spinner::new().size(16.0).color(teinte(0.5)));
        ui.label(egui::RichText::new(s).font(moyen(13.5)).color(TEXTE2));
    });
    ui.ctx().request_repaint_after(Duration::from_millis(100));
}

/// Boutons du bas, à droite : secondaire (`Some(false)`), principal vert (`Some(true)`, absent
/// si `ok` est vide, grisé si `actif` est faux).
fn boutons(ui: &mut Ui, annuler: &str, ok: &str, actif: bool) -> Option<bool> {
    ui.add_space(16.0);
    let (r, _) = ui.allocate_exact_size(vec2(LARGEUR, 38.0), Sense::hover());
    let mut res = None;
    let mut x = r.right();
    if !ok.is_empty() {
        let w = ui
            .painter()
            .layout_no_wrap(ok.into(), demi(13.5), TEXTE)
            .size()
            .x
            + 64.0;
        let b = Rect::from_min_size(pos2(x - w, r.top()), vec2(w, 38.0));
        let rep = ui
            .scope_builder(UiBuilder::new().max_rect(b), |ui| {
                ui.add_enabled_ui(actif, |ui| {
                    bouton_gel(ui, ok, Some(Icone::Telecharger), b.size(), GEL_VERT)
                })
                .inner
            })
            .inner;
        if rep.clicked() && actif {
            res = Some(true);
        }
        x = b.left() - 10.0;
    }
    let w = ui
        .painter()
        .layout_no_wrap(annuler.into(), demi(13.5), TEXTE)
        .size()
        .x
        + 44.0;
    let b = Rect::from_min_size(pos2(x - w, r.top()), vec2(w, 38.0));
    if a_la_place(ui, b, |ui| bouton_secondaire(ui, annuler, None, b.size())).clicked() {
        res = Some(false);
    }
    res
}

/// Ligne de choix (bouton radio) : orbe pleine si choisie ; grisée si indisponible.
fn choix_radio(ui: &mut Ui, actif: bool, dispo: bool, nom: &str, droite: &str) -> bool {
    let (r, rep) = ui.allocate_exact_size(
        vec2(LARGEUR, 36.0),
        if dispo {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let p = ui.painter().clone();
    let h = if dispo { survol(ui, &rep) } else { 0.0 };
    selection(&p, r, 8.0, if actif { 1.0 } else { 0.45 * h });
    let c = pos2(r.left() + 18.0, r.center().y);
    if actif {
        orbe(&p, c, 7.5, GEL_BLEU);
    } else {
        p.circle_filled(c, 7.5, blanc(230));
        p.circle_stroke(c, 7.5, Stroke::new(1.0, nuit(90)));
    }
    let col = if dispo {
        TEXTE
    } else {
        TEXTE3.gamma_multiply(0.6)
    };
    texte(
        ui,
        pos2(r.left() + 36.0, r.center().y),
        Align2::LEFT_CENTER,
        nom,
        if actif { demi(13.5) } else { moyen(13.5) },
        col,
    );
    texte(
        ui,
        r.right_center() - vec2(12.0, 0.0),
        Align2::RIGHT_CENTER,
        droite,
        regulier(12.5),
        if dispo { TEXTE2 } else { col },
    );
    ui.add_space(2.0);
    dispo && rep.clicked()
}
