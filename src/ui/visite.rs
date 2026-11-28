//! Visite guidée du premier lancement : une carte à la fois (illustration dessinée, titre,
//! texte, bouton de mise en place), flèches, points d'étape, clavier ← → / Échap.

use super::guide::{fenetre, lignes, repere};
use super::*;

/// Mise en place proposée par une carte.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Action {
    Aucune,
    /// Choix de la langue en grand sous le texte (carte de bienvenue).
    Langue,
    /// Surveillance du presse-papiers.
    Presse,
    /// Guide d'installation de l'extension (la visite attend derrière).
    Extension,
    /// Téléchargement accordé de yt-dlp / ffmpeg / deno (Windows).
    Outils,
    /// Réglages → Planification (visite en pause, pastille « Reprendre »).
    Plan,
    /// Interrupteurs sur la carte : tri par type, extraction des archives.
    Apres,
    /// Sélecteur de dossier natif.
    Dossier,
    /// Windows : liens magnet et .torrent ouverts par fluxdl.
    Torrents,
}

/// Illustration d'une carte.
#[derive(Clone, Copy)]
enum Deco {
    /// Logo et jauges de connexions.
    Logo,
    /// Fenêtre de navigateur et menu du clic droit.
    Navigateur,
    /// Grand orbe et deux petits.
    Orbes(Icone, Icone),
    /// Grand orbe et deux tuiles de fichier.
    Fichiers(TypeFichier, TypeFichier),
}

struct Carte {
    icone: Icone,
    gel: Gel,
    deco: Deco,
    titre: &'static str,
    texte: &'static str,
    action: Action,
}

static CARTES: [Carte; 10] = [
    Carte {
        icone: Icone::Telecharger,
        gel: GEL_BLEU,
        deco: Deco::Logo,
        titre: "Welcome to fluxdl",
        texte: "Each file is cut into pieces downloaded side by side, up to 32 connections at once: much faster with most servers. A stopped download resumes where it left off.",
        action: Action::Langue,
    },
    Carte {
        icone: Icone::Coller,
        gel: GEL_VERT,
        deco: Deco::Orbes(Icone::Lien, Icone::Telecharger),
        titre: "Add a link",
        texte: "Paste it in the bar at the top, drop it on the window, or simply copy it anywhere: fluxdl offers to download it.",
        action: Action::Presse,
    },
    Carte {
        icone: Icone::Lien,
        gel: GEL_BLEU,
        deco: Deco::Navigateur,
        titre: "The browser extension",
        texte: "Right-click a link to send it here, and your browser's downloads land in fluxdl on their own. Chrome, Edge, Brave and Firefox.",
        action: Action::Extension,
    },
    Carte {
        icone: Icone::Lecture,
        gel: GEL_BLEU,
        deco: Deco::Fichiers(TypeFichier::Video, TypeFichier::Document),
        titre: "YouTube, Twitch and more",
        texte: "Paste the page of a video: choose the quality, the subtitles, MP4 or MP3. Videos also download with several connections.",
        action: Action::Outils,
    },
    Carte {
        icone: Icone::Copier,
        gel: GEL_VERT,
        deco: Deco::Fichiers(TypeFichier::Video, TypeFichier::Video),
        titre: "Playlists and channels",
        texte: "Paste a playlist or a channel: tick the videos you want, they all go into a folder named after it.",
        action: Action::Aucune,
    },
    Carte {
        icone: Icone::Eclair,
        gel: GEL_BLEU,
        deco: Deco::Orbes(Icone::Horloge, Icone::Pause),
        titre: "Speed and schedule",
        texte: "Limit the speed of everything or of one download (menu next to the title). Start the queue at night, put the PC to sleep when all is done.",
        action: Action::Plan,
    },
    Carte {
        icone: Icone::Dossier,
        gel: GEL_VERT,
        deco: Deco::Fichiers(TypeFichier::Archive, TypeFichier::Document),
        titre: "After the download",
        texte: "Each new file can go into the folder of its type (Videos, Music, Archives…), and zip or 7z archives can extract themselves.",
        action: Action::Apres,
    },
    Carte {
        icone: Icone::Dossier,
        gel: GEL_BLEU,
        deco: Deco::Orbes(Icone::Telecharger, Icone::Reglages),
        titre: "Your download folder",
        texte: "New downloads are saved here:",
        action: Action::Dossier,
    },
    Carte {
        icone: Icone::Horloge,
        gel: GEL_VERT,
        deco: Deco::Orbes(Icone::Actualiser, Icone::Lien),
        titre: "History",
        texte: "Finished, failed and canceled downloads stay in History: search them, open the file or download it again in one click.",
        action: Action::Aucune,
    },
    Carte {
        icone: Icone::Aimant,
        gel: GEL_BLEU,
        deco: Deco::Fichiers(TypeFichier::Torrent, TypeFichier::Disque),
        titre: "Torrents and magnet links",
        texte: "Paste a magnet link or drop a .torrent file: pick the files to keep, fluxdl downloads them from the peers.",
        action: Action::Torrents,
    },
];

/// Visite en cours : carte affichée, pause pendant une mise en place sur une page de l'appli.
pub(super) struct Visite {
    pub(super) carte: usize,
    /// Réglages → Planification ouverts depuis une carte : pastille « Reprendre » en bas.
    pub(super) pause: bool,
    /// Dossier de téléchargement au début (« Fait » une fois changé).
    dossier: String,
}

impl Visite {
    pub(super) fn new(dossier: &str) -> Self {
        Visite {
            carte: 0,
            pause: false,
            dossier: dossier.to_string(),
        }
    }

    pub(super) fn derniere(&self) -> bool {
        self.carte + 1 == CARTES.len()
    }

    /// Carte suivante ; faux sur la dernière (rien ne bouge).
    pub(super) fn suivante(&mut self) -> bool {
        if self.derniere() {
            return false;
        }
        self.carte += 1;
        true
    }

    pub(super) fn precedente(&mut self) {
        self.carte = self.carte.saturating_sub(1);
    }

    /// Point d'étape cliqué.
    pub(super) fn aller(&mut self, k: usize) {
        self.carte = k.min(CARTES.len() - 1);
    }
}

/// Au lancement : la visite si elle n'a jamais été vue (ni terminée ni passée), y compris
/// pour une installation plus ancienne qui découvre la version (réglage absent = pas vue).
pub(super) fn au_lancement(tour_seen: bool, dossier: &str) -> Option<Visite> {
    (!tour_seen).then(|| Visite::new(dossier))
}

/// La visite se dessine : pas en pause et aucune fenêtre ouverte (guide, appairage, lien
/// passé au lancement, outils…) ; sinon elle attend, sur la même carte.
pub(super) fn visible(v: Option<&Visite>, fenetre_ouverte: bool) -> bool {
    v.is_some_and(|v| !v.pause) && !fenetre_ouverte
}

/// Texte centré, renvoyé à la ligne au-delà de `largeur` (à poser par le milieu de son haut).
fn centre(ui: &Ui, s: &str, police: egui::FontId, c: Color32, largeur: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(s.to_string(), police, c, largeur);
    job.halign = Align::Center;
    ui.painter().layout_job(job)
}

impl App {
    /// Touches de la visite, AVANT le dessin des pages : ← → et Échap ne vont pas aux champs
    /// ni au panneau de détail derrière le voile.
    pub(super) fn visite_touches(&mut self, ctx: &egui::Context) {
        if !visible(self.visite.as_ref(), self.modale.is_some()) {
            return;
        }
        let (g, d, e) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
            )
        });
        if e {
            self.finir_visite();
        } else if let Some(v) = &mut self.visite {
            if g {
                v.precedente();
            }
            if d {
                v.suivante();
            }
        }
    }

    /// Fin de la visite (« C'est parti », « Passer », Échap) : vue pour de bon, écrit tout de
    /// suite. La bulle « Installer l'extension » n'est plus proposée : la visite l'a montrée.
    pub(super) fn finir_visite(&mut self) {
        self.visite = None;
        self.settings.tour_seen = true;
        self.settings.extension_asked = true;
        if self.capture.is_none() {
            store::save(SETTINGS, &self.settings);
        }
    }

    /// Langue choisie dans la visite : même réglage que Réglages → Interface → Langue, écrit
    /// tout de suite ; la visite reste sur sa carte.
    pub(super) fn choisir_langue(&mut self, lang: Lang) {
        if self.settings.lang == lang {
            return;
        }
        self.settings.lang = lang;
        if self.capture.is_none() {
            store::save(SETTINGS, &self.settings);
        }
    }

    /// Réglages → « Revoir la visite guidée » : reprend une visite en pause, sinon depuis le
    /// début.
    pub(super) fn revoir_visite(&mut self) {
        match &mut self.visite {
            Some(v) => v.pause = false,
            None => self.visite = Some(Visite::new(&self.settings.folder)),
        }
    }

    /// Mise en place demandée par une carte.
    pub(super) fn agir_visite(&mut self, a: Action) {
        match a {
            Action::Presse => {
                self.settings.clipboard_watch = true;
                self.presse_actif.store(true, Ordering::Relaxed);
            }
            Action::Extension => self.ouvrir_guide(),
            Action::Outils => self.ouvrir_video_outils(),
            Action::Plan => {
                self.tab = Tab::Settings;
                self.vers_plan = true;
                if let Some(v) = &mut self.visite {
                    v.pause = true;
                }
            }
            Action::Dossier => {
                if self.picker.is_none() {
                    self.picker_cat = None;
                    self.picker = Some(choose_folder(&self.settings.folder));
                }
            }
            Action::Torrents => {
                self.settings.handler_asked = true;
                self.settings.torrent_handler = torrent::associer(true);
            }
            Action::Aucune | Action::Langue | Action::Apres => {}
        }
    }

    /// Mise en place déjà faite : coche « Fait » à la place du bouton.
    fn fait(&self, a: Action) -> bool {
        match a {
            Action::Presse => self.settings.clipboard_watch,
            Action::Extension => crate::extension::dossier().join("manifest.json").exists(),
            Action::Outils => self.outils.complets(),
            Action::Plan => self.settings.plan_start.is_some(),
            Action::Dossier => self
                .visite
                .as_ref()
                .is_some_and(|v| v.dossier != self.settings.folder),
            Action::Torrents => self.settings.torrent_handler,
            Action::Aucune | Action::Langue | Action::Apres => false,
        }
    }

    /// Voile, fenêtre de la visite et pastille « Reprendre » (visite en pause).
    pub(super) fn visite(&mut self, ctx: &egui::Context) {
        let montre = visible(self.visite.as_ref(), self.modale.is_some());
        let ecran = ctx.content_rect();
        // Voile en fondu (0,25 s), qui bloque les clics vers l'appli derrière.
        let v = anim::bool(ctx, egui::Id::unique("visite-voile"), montre, 0.25, 0.15);
        if v > 0.0 {
            egui::Area::new(egui::Id::unique("visite-voile"))
                .order(egui::Order::Middle)
                .fixed_pos(ecran.min)
                .fade_in(false)
                .show(ctx, |ui| {
                    let sens = if montre {
                        Sense::click()
                    } else {
                        Sense::hover()
                    };
                    let (r, _) = ui.allocate_exact_size(ecran.size(), sens);
                    ui.painter().rect_filled(r, 0.0, nuit((110.0 * v) as u8));
                });
        }
        if !montre {
            self.pastille_visite(ctx);
            return;
        }
        let Some(vis) = &self.visite else {
            return;
        };
        let (carte, derniere) = (vis.carte, vis.derniere());
        let l = self.settings.lang;
        let n = CARTES.len();
        // Glissement entre deux cartes (0,4 s, sortie cubique ; immédiat si animations réduites).
        let pos = anim::valeur(ctx, egui::Id::unique("visite-glisse"), carte as f32, 0.4);
        // Ouverture : fondu et échelle 0,96 → 1, comme les fenêtres.
        let a = anim::impulsion(ctx, egui::Id::unique("visite-ouvre"), true, 0.3, true)
            .map_or(1.0, anim::sortie_quintique);
        let couche = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("visite"));
        let c = ecran.center().to_vec2();
        ctx.set_transform_layer(
            couche,
            egui::emath::TSTransform::from_translation(c)
                * egui::emath::TSTransform::from_scaling(0.96 + 0.04 * a)
                * egui::emath::TSTransform::from_translation(-c),
        );
        let taille = vec2(
            (ecran.width() - 48.0).min(640.0),
            (ecran.height() - 48.0).min(480.0),
        );
        let mut action = None;
        let mut aller = None;
        let mut fin = false;
        let mut lang = l;
        egui::Area::new(egui::Id::unique("visite"))
            .order(egui::Order::Foreground)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .fade_in(false)
            .show(ctx, |ui| {
                ui.multiply_opacity(a);
                let (r, _) = ui.allocate_exact_size(taille, Sense::click());
                let p = ui.painter().clone();
                p.add(lueur(
                    r.expand(2.0),
                    rgb(0x5c, 0xc8, 0xff).gamma_multiply(0.35),
                    18.0,
                ));
                p.add(verre(ui.ctx(), r, 18, 0.88));
                // Haut : étape, « Passer » toujours visible.
                texte(
                    ui,
                    pos2(r.left() + 24.0, r.top() + 30.0),
                    Align2::LEFT_CENTER,
                    &format!("{} · {} / {n}", t(l, "Guided tour"), carte + 1),
                    moyen(12.5),
                    TEXTE3,
                );
                let txt = t(l, "Skip");
                // Largeur de la plus longue traduction : rien ne bouge quand la langue change.
                let w = [Lang::Fr, Lang::En]
                    .map(|k| {
                        p.layout_no_wrap(t(k, "Skip").into(), demi(13.5), TEXTE)
                            .size()
                            .x
                    })
                    .into_iter()
                    .fold(0.0, f32::max)
                    + 36.0;
                let bp =
                    Rect::from_min_size(pos2(r.right() - 16.0 - w, r.top() + 15.0), vec2(w, 30.0));
                if a_la_place(ui, bp, |ui| bouton_secondaire(ui, txt, None, bp.size())).clicked() {
                    fin = true;
                }
                // Langue, sur toutes les cartes, à gauche de « Passer ».
                let bl = Rect::from_min_size(
                    pos2(bp.left() - 10.0 - 90.0, r.top() + 17.0),
                    vec2(90.0, 26.0),
                );
                ui.scope_builder(
                    UiBuilder::new().max_rect(bl).id_salt("visite-langue"),
                    |ui| {
                        segmente_taille(
                            ui,
                            &mut lang,
                            &[(Lang::Fr, "FR"), (Lang::En, "EN")],
                            42.0,
                            26.0,
                            12.0,
                        )
                        .on_hover_text("Langue / Language")
                    },
                );
                // Cartes : celle d'avant et la suivante pendant le glissement.
                let zone = Rect::from_min_max(
                    pos2(r.left() + 1.0, r.top() + 54.0),
                    pos2(r.right() - 1.0, r.bottom() - 72.0),
                );
                let (k0, k1) = (pos.floor() as usize, pos.ceil() as usize);
                for k in if k0 == k1 { vec![k0] } else { vec![k0, k1] } {
                    let d = k as f32 - pos;
                    let o = (1.0 - d.abs() * 1.2).clamp(0.0, 1.0);
                    let zk = zone.translate(vec2(d * r.width() * 0.75, 0.0));
                    if let Some(x) = self.carte_visite(ui, k, zk, zone, o) {
                        action = Some(x);
                    }
                }
                // Bas : flèches, points d'étape (le point bleu glisse avec la carte).
                let y = r.bottom() - 38.0;
                let bg = Rect::from_center_size(pos2(r.left() + 44.0, y), Vec2::splat(42.0));
                let prec = ui
                    .scope_builder(UiBuilder::new().max_rect(bg), |ui| {
                        ui.add_enabled_ui(carte > 0, |ui| {
                            bouton_fleche(ui, 42.0, false, t(l, "Previous"))
                        })
                        .inner
                    })
                    .inner;
                if prec.clicked() {
                    aller = Some(carte.saturating_sub(1));
                }
                if derniere {
                    let txt = t(l, "Let's go");
                    let w = p.layout_no_wrap(txt.into(), demi(13.5), TEXTE).size().x + 64.0;
                    let b =
                        Rect::from_min_size(pos2(r.right() - 22.0 - w, y - 20.0), vec2(w, 40.0));
                    let rep = a_la_place(ui, b, |ui| {
                        bouton_gel_ouvert(ui, txt, Some(Icone::Lecture), b.size(), GEL_VERT, true)
                    });
                    if rep.clicked() {
                        fin = true;
                    }
                } else {
                    let bd = Rect::from_center_size(pos2(r.right() - 44.0, y), Vec2::splat(42.0));
                    if a_la_place(ui, bd, |ui| bouton_fleche(ui, 42.0, true, t(l, "Next")))
                        .clicked()
                    {
                        aller = Some(carte + 1);
                    }
                }
                let pas = 18.0;
                let x0 = r.center().x - pas * (n as f32 - 1.0) / 2.0;
                for (k, carte) in CARTES.iter().enumerate() {
                    let c = pos2(x0 + pas * k as f32, y);
                    let rep = ui
                        .interact(
                            Rect::from_center_size(c, Vec2::splat(16.0)),
                            egui::Id::unique(("visite-point", k)),
                            Sense::click(),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(t(l, carte.titre));
                    let h = survol(ui, &rep);
                    p.circle_filled(c, 4.0 + h, nuit(45 + (40.0 * h) as u8));
                    if rep.clicked() {
                        aller = Some(k);
                    }
                }
                orbe(&p, pos2(x0 + pas * pos, y), 6.0, GEL_BLEU);
            });
        if let Some(k) = aller {
            if let Some(v) = &mut self.visite {
                v.aller(k);
            }
        }
        if let Some(x) = action {
            self.agir_visite(x);
        }
        if lang != l {
            self.choisir_langue(lang);
        }
        if fin {
            self.finir_visite();
        }
    }

    /// Une carte dans `zk` (découpée par `zone`), opacité `o` ; rend l'action cliquée.
    fn carte_visite(
        &mut self,
        ui: &mut Ui,
        k: usize,
        zk: Rect,
        zone: Rect,
        o: f32,
    ) -> Option<Action> {
        let l = self.settings.lang;
        let c = &CARTES[k];
        let mut p = ui.painter().with_clip_rect(zone);
        p.multiply_opacity(o);
        let ih = (zk.height() * 0.46).clamp(100.0, 176.0);
        let cx = zk.center().x;
        let large = zk.width() - 100.0;
        let titre = centre(ui, t(l, c.titre), demi(22.0), TITRE, large);
        let corps = centre(ui, t(l, c.texte), regulier(14.5), TEXTE2, large);
        // Trop long : la fin du chemin reste visible.
        let chemin = (c.action == Action::Dossier).then(|| {
            let chemin = &self.settings.folder;
            let mut debut = 0;
            let mut g = p.layout_no_wrap(chemin.clone(), moyen(13.5), TEXTE);
            while g.size().x > large && debut + 1 < chemin.chars().count() {
                debut += 1;
                let fin: String = chemin.chars().skip(debut).collect();
                g = p.layout_no_wrap(format!("…{fin}"), moyen(13.5), TEXTE);
            }
            g
        });
        let note = derniere_carte(k).then(|| {
            centre(
                ui,
                t(l, "You can replay this tour from Settings."),
                regulier(12.5),
                TEXTE3,
                large,
            )
        });
        let fait = self.fait(c.action);
        let linux = c.action == Action::Outils && !cfg!(windows) && !fait;
        let sans_bouton =
            c.action == Action::Aucune || (c.action == Action::Torrents && !cfg!(windows) && !fait);
        let ha = match c.action {
            _ if sans_bouton => 0.0,
            Action::Apres => 72.0,
            Action::Langue => 78.0,
            _ if linux => 34.0,
            _ => 50.0,
        };
        // Contenu centré dans la hauteur de la carte.
        let total = ih
            + 6.0
            + titre.size().y
            + 8.0
            + corps.size().y
            + chemin.as_ref().map_or(0.0, |g| g.size().y + 6.0)
            + note.as_ref().map_or(0.0, |g| g.size().y + 6.0)
            + ha;
        let y0 = zk.top() + ((zk.height() - total) / 2.0).max(0.0);
        self.illustration(
            &p,
            Rect::from_min_size(pos2(zk.left(), y0), vec2(zk.width(), ih)),
            c,
        );
        let mut y = y0 + ih + 6.0;
        let h = titre.size().y;
        p.galley(pos2(cx, y), titre, TITRE);
        y += h + 8.0;
        let h = corps.size().y;
        p.galley(pos2(cx, y), corps, TEXTE2);
        y += h + 6.0;
        if let Some(g) = chemin {
            let h = g.size().y;
            p.galley(pos2(cx - g.size().x / 2.0, y), g, TEXTE);
            y += h + 6.0;
        }
        if let Some(g) = note {
            let h = g.size().y;
            p.galley(pos2(cx, y), g, TEXTE3);
            y += h + 6.0;
        }
        // Mise en place : bouton, coche « Fait », ou interrupteurs sur la carte.
        let y = y + 6.0;
        let (libelle, icone) = match c.action {
            Action::Presse => (t(l, "Watch the clipboard"), Icone::Coller),
            Action::Extension => (t(l, "Install the extension"), Icone::Telecharger),
            Action::Outils if cfg!(windows) => {
                (t(l, "Install the video tools"), Icone::Telecharger)
            }
            Action::Outils => {
                if !fait {
                    let g = centre(
                        ui,
                        t(
                            l,
                            "Needs yt-dlp and ffmpeg: install them with your package manager.",
                        ),
                        regulier(13.0),
                        TEXTE3,
                        large,
                    );
                    p.galley(pos2(cx, y + 4.0), g, TEXTE3);
                    return None;
                }
                ("", Icone::Telecharger)
            }
            Action::Plan => (t(l, "Open scheduling"), Icone::Horloge),
            Action::Dossier => (t(l, "Change the folder"), Icone::Dossier),
            Action::Torrents if cfg!(windows) => {
                (t(l, "Open magnet links with fluxdl"), Icone::Aimant)
            }
            Action::Langue => {
                p.text(
                    pos2(cx, y + 8.0),
                    Align2::CENTER_CENTER,
                    "Langue / Language",
                    moyen(12.5),
                    TEXTE3,
                );
                let b = Rect::from_center_size(pos2(cx, y + 44.0), vec2(266.0, 40.0));
                let mut lang = l;
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(b)
                        .id_salt(("visite-langue-carte", k)),
                    |ui| {
                        ui.set_clip_rect(zone);
                        ui.multiply_opacity(o);
                        segmente_taille(
                            ui,
                            &mut lang,
                            &[(Lang::Fr, "Français"), (Lang::En, "English")],
                            130.0,
                            40.0,
                            14.5,
                        );
                    },
                );
                self.choisir_langue(lang);
                return None;
            }
            Action::Apres => {
                let inters = [
                    (t(l, "Sort by file type"), &mut self.settings.sort_by_type),
                    (
                        t(l, "Extract archives automatically"),
                        &mut self.settings.auto_extract,
                    ),
                ];
                let w = inters
                    .iter()
                    .map(|(s, _)| {
                        p.layout_no_wrap(s.to_string(), regulier(13.5), TEXTE)
                            .size()
                            .x
                    })
                    .fold(0.0, f32::max)
                    + 56.0;
                for (i, (s, on)) in inters.into_iter().enumerate() {
                    let b =
                        Rect::from_min_size(pos2(cx - w / 2.0, y + i as f32 * 34.0), vec2(w, 26.0));
                    ui.scope_builder(UiBuilder::new().max_rect(b), |ui| {
                        ui.set_clip_rect(zone);
                        ui.multiply_opacity(o);
                        ui.horizontal(|ui| {
                            interrupteur(ui, on);
                            ui.add_space(4.0);
                            ui.label(egui::RichText::new(s).font(regulier(13.5)).color(TEXTE));
                        });
                    });
                }
                return None;
            }
            _ => return None,
        };
        if fait {
            let txt = if c.action == Action::Outils {
                t(l, "Video tools ready")
            } else {
                t(l, "Done")
            };
            let g = p.layout_no_wrap(txt.into(), demi(14.0), SUCCES);
            let w = g.size().x + 30.0;
            let x = cx - w / 2.0;
            coche_ronde(&p, pos2(x + 10.0, y + 19.0), 10.0);
            p.galley(pos2(x + 28.0, y + 19.0 - g.size().y / 2.0), g, SUCCES);
            return None;
        }
        let w = p.layout_no_wrap(libelle.into(), demi(13.5), TEXTE).size().x + 72.0;
        let b = Rect::from_min_size(pos2(cx - w / 2.0, y), vec2(w, 38.0));
        let rep = ui
            .scope_builder(UiBuilder::new().max_rect(b), |ui| {
                ui.set_clip_rect(zone);
                ui.multiply_opacity(o);
                bouton_gel(ui, libelle, Some(icone), b.size(), GEL_BLEU)
            })
            .inner;
        rep.clicked().then_some(c.action)
    }

    /// Illustration d'une carte, dessinée au pinceau dans le style des icônes (orbes, gel).
    fn illustration(&self, p: &egui::Painter, r: Rect, c: &Carte) {
        let s = (r.height() / 170.0).min(1.0);
        let m = r.center() + vec2(0.0, 4.0 * s);
        // Bulles de savon autour.
        for (dx, dy, rr) in [
            (-205.0, -38.0, 7.0),
            (-160.0, 46.0, 4.0),
            (-232.0, 18.0, 3.0),
            (172.0, -52.0, 5.0),
            (214.0, 26.0, 8.0),
            (128.0, 64.0, 3.5),
        ] {
            let b = m + vec2(dx, dy) * s;
            let rr = rr * s;
            p.circle_filled(b, rr, rgb(0x9a, 0xdc, 0xff).gamma_multiply(0.35));
            p.circle_stroke(b, rr, Stroke::new(1.0, blanc(200)));
            p.circle_filled(b - vec2(rr, rr) * 0.35, rr * 0.3, blanc(220));
        }
        let grand = |rayon: f32| {
            let rayon = rayon * s;
            p.add(lueur(
                Rect::from_center_size(m, Vec2::splat(rayon * 2.2)),
                c.gel.lueur.gamma_multiply(0.5),
                rayon * 0.7,
            ));
            p.circle_filled(m + vec2(0.0, rayon * 0.12), rayon + 1.0, nuit(40));
            orbe(p, m, rayon, c.gel);
            icone_relief(
                p,
                Rect::from_center_size(m, Vec2::splat(rayon * 0.95)),
                c.icone,
            );
        };
        match c.deco {
            Deco::Logo => {
                // Jauges de connexions de part et d'autre du logo.
                for (i, frac) in [0.92, 0.64, 0.8, 0.45, 0.7, 0.86, 0.55, 0.74]
                    .into_iter()
                    .enumerate()
                {
                    let gauche = i < 4;
                    let j = (i % 4) as f32;
                    let x = if gauche {
                        m.x - 196.0 * s
                    } else {
                        m.x + 92.0 * s
                    };
                    let jr = Rect::from_min_size(
                        pos2(x, m.y - 34.0 * s + j * 20.0 * s),
                        vec2(104.0 * s, 9.0 * s),
                    );
                    jauge(p, jr, frac, couleur_connexion(i, 8), false);
                }
                let lr = Rect::from_center_size(m, Vec2::splat(124.0 * s));
                p.add(lueur(
                    lr.shrink(10.0 * s),
                    rgb(0x5c, 0xc8, 0xff).gamma_multiply(0.5),
                    30.0 * s,
                ));
                p.image(
                    self.logo.id(),
                    lr,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            Deco::Navigateur => {
                let w = Rect::from_center_size(m, vec2(300.0 * s, 136.0 * s));
                let dedans = fenetre(p, w, "exemple.fr/telechargements");
                lignes(
                    p,
                    dedans.left() + 16.0,
                    dedans.top() + 14.0,
                    &[120.0 * s, 170.0 * s, 90.0 * s],
                );
                // Lien souligné, puis menu du clic droit avec la ligne fluxdl choisie.
                let lien = Rect::from_min_size(
                    pos2(dedans.left() + 16.0, dedans.top() + 44.0 * s),
                    vec2(96.0 * s, 4.0),
                );
                p.rect_filled(lien, 2.0, ACCENT);
                let menu = Rect::from_min_size(
                    pos2(lien.left() + 70.0 * s, lien.bottom() + 6.0),
                    vec2(150.0 * s, 58.0 * s),
                );
                p.add(lueur(menu.translate(vec2(0.0, 2.0)), nuit(50), 6.0));
                p.rect_filled(menu, 5.0, rgb(0xfb, 0xfc, 0xfe));
                p.rect_stroke(menu, 5.0, Stroke::new(1.0, nuit(70)), StrokeKind::Inside);
                let h = menu.height() / 3.0;
                for i in 0..3 {
                    let li = Rect::from_min_size(
                        pos2(menu.left() + 4.0, menu.top() + 2.0 + i as f32 * (h - 1.0)),
                        vec2(menu.width() - 8.0, h - 2.0),
                    );
                    if i == 1 {
                        selection(p, li, 3.0, 1.0);
                        let lo = Rect::from_center_size(
                            pos2(li.left() + 10.0, li.center().y),
                            Vec2::splat((h - 6.0).min(14.0)),
                        );
                        p.image(
                            self.logo.id(),
                            lo,
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                        p.text(
                            pos2(lo.right() + 5.0, li.center().y),
                            Align2::LEFT_CENTER,
                            "fluxdl",
                            demi(9.5 * s.max(0.8)),
                            TEXTE,
                        );
                        repere(p, li, 3.0);
                    } else {
                        lignes(p, li.left() + 8.0, li.center().y - 1.5, &[80.0 * s]);
                    }
                }
                // Petit orbe de l'appli qui reçoit le lien.
                let o = pos2(w.right() + 52.0 * s, m.y + 4.0 * s);
                p.circle_filled(o + vec2(0.0, 4.0 * s), 31.0 * s, nuit(35));
                orbe(p, o, 30.0 * s, GEL_VERT);
                icone_relief(
                    p,
                    Rect::from_center_size(o, Vec2::splat(28.0 * s)),
                    Icone::Telecharger,
                );
            }
            Deco::Orbes(a, b) => {
                for (dx, ic, t) in [(-128.0, a, 0.25), (128.0, b, 0.8)] {
                    let o = m + vec2(dx, 22.0) * s;
                    let rr = 27.0 * s;
                    p.circle_filled(o + vec2(0.0, 3.0 * s), rr + 1.0, nuit(35));
                    orbe(p, o, rr, gel_de(teinte(t)));
                    icone_relief(p, Rect::from_center_size(o, Vec2::splat(rr * 0.95)), ic);
                }
                grand(54.0);
            }
            Deco::Fichiers(a, b) => {
                for (dx, ty) in [(-130.0, a), (130.0, b)] {
                    let o = m + vec2(dx, 20.0) * s;
                    tuile_fichier(p, Rect::from_center_size(o, Vec2::splat(54.0 * s)), ty);
                }
                grand(54.0);
            }
        }
    }

    /// Visite en pause (page des réglages ouverte depuis une carte) : « Reprendre » en bas.
    fn pastille_visite(&mut self, ctx: &egui::Context) {
        if !self.visite.as_ref().is_some_and(|v| v.pause) || self.modale.is_some() {
            return;
        }
        let l = self.settings.lang;
        let ecran = ctx.content_rect();
        let taille = vec2(340.0, 52.0);
        let pos = pos2(
            ecran.center().x - taille.x / 2.0,
            ecran.bottom() - taille.y - 24.0,
        );
        let mut reprendre = false;
        egui::Area::new(egui::Id::unique("visite-pause"))
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
                p.add(verre(ui.ctx(), r, 16, 0.86));
                let txt = t(l, "Resume");
                let w = p.layout_no_wrap(txt.into(), demi(13.5), TEXTE).size().x + 60.0;
                let b =
                    Rect::from_min_size(pos2(r.right() - 10.0 - w, r.top() + 10.0), vec2(w, 32.0));
                let g = ligne(
                    ui,
                    t(l, "Guided tour paused"),
                    demi(13.5),
                    TITRE,
                    b.left() - r.left() - 30.0,
                );
                p.galley(
                    pos2(r.left() + 18.0, r.center().y - g.size().y / 2.0),
                    g,
                    TITRE,
                );
                reprendre = a_la_place(ui, b, |ui| {
                    bouton_principal(ui, txt, Some(Icone::Lecture), b.size())
                })
                .clicked();
            });
        if reprendre {
            self.revoir_visite();
        }
    }
}

fn derniere_carte(k: usize) -> bool {
    k + 1 == CARTES.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Suivant / précédent / points : bornes ; la dernière carte ne va pas plus loin.
    #[test]
    fn navigation() {
        let mut v = Visite::new("/dl");
        v.precedente();
        assert_eq!(v.carte, 0);
        for i in 1..CARTES.len() {
            assert!(v.suivante());
            assert_eq!(v.carte, i);
        }
        assert!(v.derniere());
        assert!(!v.suivante());
        assert_eq!(v.carte, CARTES.len() - 1);
        v.precedente();
        assert_eq!(v.carte, CARTES.len() - 2);
        v.aller(99);
        assert_eq!(v.carte, CARTES.len() - 1);
        v.aller(3);
        assert_eq!(v.carte, 3);
    }

    /// Affichée une fois ; attend derrière une fenêtre ouverte ou pendant la pause.
    #[test]
    fn conditions_d_affichage() {
        assert!(au_lancement(true, "/dl").is_none());
        let mut v = au_lancement(false, "/dl").unwrap();
        assert_eq!(v.carte, 0);
        assert!(visible(Some(&v), false));
        assert!(!visible(Some(&v), true));
        assert!(!visible(None, false));
        v.pause = true;
        assert!(!visible(Some(&v), false));
    }

    /// Toutes les phrases de la visite sont traduites.
    #[test]
    fn cartes_traduites() {
        for c in &CARTES {
            for s in [c.titre, c.texte] {
                assert_ne!(t(Lang::Fr, s), s, "{s}");
            }
        }
    }
}
