//! Installation guidée de l'extension (fenêtre en 3 étapes illustrées, bulle du premier
//! lancement) et fenêtre « Autoriser ? » de l'appairage automatique.

use super::fenetres::{boutons, paragraphe, titre_modale};
use super::*;
use crate::extension::{self, Navigateur};
use eframe::egui::{CornerRadius, Pos2, Shape};
use std::time::Instant;

/// Largeur de la fenêtre du guide (plus large que les autres : dessins + textes).
pub(super) const LARGEUR_GUIDE: f32 = 640.0;
/// Contour des éléments à cliquer dans les dessins.
const REPERE: Color32 = rgb(0xff, 0x8a, 0x00);

/// Un onglet du guide : navigateur trouvé, ou variante sans exécutable (dossier seulement).
pub(super) struct Onglet {
    nom: &'static str,
    nav: Option<Navigateur>,
    firefox: bool,
}

pub(super) struct Guide {
    onglets: Vec<Onglet>,
    choix: usize,
    /// Dossier décompressé (Chrome…) et zip Firefox (écrit au clic).
    dossier: PathBuf,
    zip: PathBuf,
    erreur: Option<String>,
    /// Page des extensions demandée au navigateur (rappel de l'adresse à taper sinon).
    ouvert: bool,
}

impl Guide {
    pub(super) fn new(
        navs: Vec<Navigateur>,
        dossier: PathBuf,
        zip: PathBuf,
        erreur: Option<String>,
    ) -> Self {
        let mut onglets: Vec<Onglet> = navs
            .into_iter()
            .map(|n| Onglet {
                nom: n.nom,
                firefox: n.firefox,
                nav: Some(n),
            })
            .collect();
        if !onglets.iter().any(|o| !o.firefox) {
            onglets.insert(
                0,
                Onglet {
                    nom: "Chrome / Edge",
                    nav: None,
                    firefox: false,
                },
            );
        }
        if !onglets.iter().any(|o| o.firefox) {
            onglets.push(Onglet {
                nom: "Firefox",
                nav: None,
                firefox: true,
            });
        }
        // Six onglets tiennent dans la fenêtre : Firefox reste le dernier.
        while onglets.len() > 6 {
            onglets.remove(onglets.len() - 2);
        }
        Guide {
            onglets,
            choix: 0,
            dossier,
            zip,
            erreur,
            ouvert: false,
        }
    }
}

impl App {
    /// Réglages → Installer, ou bulle du premier lancement : extension décompressée (à jour
    /// avec l'appli), navigateurs détectés, guide.
    pub(super) fn ouvrir_guide(&mut self) {
        self.settings.extension_asked = true;
        let dossier = extension::dossier();
        let erreur = extension::extraire(&dossier).err().map(|e| e.to_string());
        let zip = PathBuf::from(&self.settings.folder).join(format!(
            "{APP_NAME}-firefox-{}.zip",
            env!("CARGO_PKG_VERSION")
        ));
        self.modale = Some(Modale::Guide(Guide::new(
            extension::detecter(),
            dossier,
            zip,
            erreur,
        )));
    }

    /// Demande d'appairage du serveur : fenêtre « Autoriser ? » (remplace le guide, attend
    /// la fermeture des autres fenêtres) ; fermée d'elle-même après 60 s.
    pub(super) fn suivre_appairage(&mut self, ctx: &egui::Context) {
        let maintenant = Instant::now();
        let p = self.appairage.lock().unwrap_or_else(|e| e.into_inner());
        match (p.en_cours(maintenant), &self.modale) {
            (Some(a), Some(Modale::Appairage { depuis, .. })) if *depuis == a.depuis => {}
            (Some(a), None | Some(Modale::Guide(_)) | Some(Modale::Appairage { .. })) => {
                self.modale = Some(Modale::Appairage {
                    depuis: a.depuis,
                    origine: a.origine.clone(),
                    navigateur: a.navigateur,
                });
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            (None, Some(Modale::Appairage { .. })) => self.modale = None,
            _ => {}
        }
    }

    /// Réponse à la demande montrée (Échap ou clic à côté = refus).
    pub(super) fn repondre_appairage(&self, depuis: Instant, oui: bool) {
        self.appairage
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .repondre(depuis, oui, Instant::now());
    }

    /// « Autoriser ? » ; vrai = fermer.
    pub(super) fn contenu_appairage(
        &mut self,
        ui: &mut Ui,
        depuis: Instant,
        origine: String,
        navigateur: &'static str,
    ) -> bool {
        let l = self.settings.lang;
        titre_modale(ui, Icone::Lien, t(l, "Connect the extension?"));
        paragraphe(
            ui,
            &format!(
                "{} ({navigateur}, {} {origine}). {}",
                t(l, "A fluxdl extension asks for access"),
                t(l, "origin"),
                t(l, "Allow?")
            ),
        );
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(t(
                l,
                "Allow only if you just installed it or clicked “Connect to fluxdl”. It will be able to send downloads to fluxdl.",
            ))
            .font(regulier(12.5))
            .color(TEXTE3),
        );
        match boutons(ui, t(l, "Deny"), t(l, "Allow"), true) {
            Some(oui) => {
                self.repondre_appairage(depuis, oui);
                true
            }
            None => {
                self.modale = Some(Modale::Appairage {
                    depuis,
                    origine,
                    navigateur,
                });
                false
            }
        }
    }

    /// Guide en 3 étapes ; vrai = fermer.
    pub(super) fn contenu_guide(&mut self, ui: &mut Ui, mut g: Guide) -> bool {
        let l = self.settings.lang;
        let w = ui.available_width();
        titre_modale(
            ui,
            Icone::Telecharger,
            t(l, "Install the browser extension"),
        );
        let options: Vec<(usize, &str)> = g
            .onglets
            .iter()
            .enumerate()
            .map(|(i, o)| (i, o.nom))
            .collect();
        if segmente(ui, &mut g.choix, &options).changed() {
            g.ouvert = false;
        }
        ui.add_space(14.0);
        let o = &g.onglets[g.choix];
        let firefox = o.firefox;
        let chemin = if firefox { &g.zip } else { &g.dossier };
        let chemin_txt = chemin.display().to_string();
        // Chemin (lecture seule), Copier, Ouvrir le navigateur (ou le dossier).
        let ouvrir = match &o.nav {
            Some(n) => format!("{} {}", t(l, "Open"), n.nom),
            None => t(l, "Open the folder").to_string(),
        };
        let lo = ui
            .painter()
            .layout_no_wrap(ouvrir.clone(), demi(13.5), TEXTE)
            .size()
            .x
            + 64.0;
        let lc = ui
            .painter()
            .layout_no_wrap(t(l, "Copy the path").into(), demi(13.5), TEXTE)
            .size()
            .x
            + 64.0;
        let (rang, _) = ui.allocate_exact_size(vec2(w, 38.0), Sense::hover());
        let bo = Rect::from_min_size(pos2(rang.right() - lo, rang.top()), vec2(lo, 38.0));
        let bc = Rect::from_min_size(pos2(bo.left() - 10.0 - lc, rang.top()), vec2(lc, 38.0));
        let boite = Rect::from_min_max(rang.min, pos2(bc.left() - 10.0, rang.bottom()));
        let p = ui.painter().clone();
        p.rect_filled(boite, 8.0, blanc(235));
        p.rect_stroke(boite, 8.0, Stroke::new(1.0, BORD_FORT), StrokeKind::Inside);
        // Trop long : la fin du chemin (dossier de l'extension, nom du zip) reste visible.
        let mut debut = 0;
        let mut gal = p.layout_no_wrap(chemin_txt.clone(), regulier(12.5), TEXTE);
        while gal.size().x > boite.width() - 20.0 && debut + 1 < chemin_txt.chars().count() {
            debut += 1;
            let fin: String = chemin_txt.chars().skip(debut).collect();
            gal = p.layout_no_wrap(format!("…{fin}"), regulier(12.5), TEXTE);
        }
        p.galley(
            pos2(boite.left() + 10.0, boite.center().y - gal.size().y / 2.0),
            gal,
            TEXTE,
        );
        let copier = a_la_place(ui, bc, |ui| {
            bouton_secondaire(ui, t(l, "Copy the path"), Some(Icone::Copier), bc.size())
        })
        .clicked();
        let lancer = a_la_place(ui, bo, |ui| {
            let i = if o.nav.is_some() {
                Icone::Lien
            } else {
                Icone::Dossier
            };
            bouton_gel(ui, &ouvrir, Some(i), bo.size(), GEL_BLEU)
        })
        .clicked();
        if (copier || lancer) && firefox {
            if let Err(e) = extension::ecrire_zip_firefox(g.zip.parent().unwrap_or(Path::new(".")))
            {
                g.erreur = Some(e.to_string());
            }
        }
        if copier || lancer {
            ui.ctx().copy_text(chemin_txt.clone());
        }
        if lancer {
            match &o.nav {
                Some(n) => {
                    if !extension::ouvrir(n) {
                        g.erreur = Some(format!("{} {}", t(l, "Cannot start"), n.nom));
                    }
                    g.ouvert = true;
                }
                None => open_folder(chemin.parent().filter(|_| firefox).unwrap_or(chemin)),
            }
        }
        // Rappel : l'adresse interne, si le navigateur ne l'a pas ouverte.
        let rappel = match &o.nav {
            Some(n) if g.ouvert => format!(
                "{} {} {}",
                t(l, "Path copied. If the page did not open, type"),
                n.page,
                t(l, "in the address bar.")
            ),
            Some(n) => format!(
                "{} {} {}",
                t(l, "Opens"),
                n.page,
                t(l, "and copies the path.")
            ),
            None if firefox => t(
                l,
                "Firefox not found: the button saves the zip in this folder; open about:debugging in Firefox.",
            )
            .into(),
            None => t(l, "No browser found: open its extensions page yourself.").into(),
        };
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(rappel)
                .font(regulier(12.5))
                .color(TEXTE3),
        );
        ui.add_space(12.0);
        let etapes: [(&str, &str); 3] = if firefox {
            [
                (
                    "Open “This Firefox”",
                    "On the left of the about:debugging page.",
                ),
                (
                    "Click “Load Temporary Add-on…”",
                    "The button is at the top of the page.",
                ),
                (
                    "Pick the zip file",
                    "Ctrl+V pastes its path (already copied), then Open.",
                ),
            ]
        } else {
            [
                (
                    "Turn on “Developer mode”",
                    "Top right of the extensions page (Edge: bottom left).",
                ),
                (
                    "Click “Load unpacked”",
                    "The button appears under the page title.",
                ),
                (
                    "Paste the path and confirm",
                    "Ctrl+V in the folder picker (path already copied), then Select Folder.",
                ),
            ]
        };
        for (i, (titre, aide)) in etapes.into_iter().enumerate() {
            let (r, _) = ui.allocate_exact_size(vec2(w, 92.0), Sense::hover());
            let d = Rect::from_min_size(r.min + vec2(0.0, 2.0), vec2(168.0, 88.0));
            dessin(&p, d, i, firefox, l);
            let x = d.right() + 18.0;
            let c = pos2(x + 13.0, r.top() + 24.0);
            orbe(&p, c, 13.0, GEL_BLEU);
            let n = p.layout_no_wrap((i + 1).to_string(), demi(13.5), Color32::WHITE);
            p.galley(c - n.size() / 2.0, n, Color32::WHITE);
            let xt = x + 36.0;
            // Titre sur une ou deux lignes, aide en dessous.
            let job = |s: &str, f, c| egui::text::LayoutJob::simple(s.into(), f, c, r.right() - xt);
            let g1 = p.layout_job(job(t(l, titre), demi(14.5), TITRE));
            let y = c.y - 10.0;
            let h = g1.size().y;
            p.galley(pos2(xt, y), g1, TITRE);
            p.galley(
                pos2(xt, y + h + 3.0),
                p.layout_job(job(t(l, aide), regulier(12.5), TEXTE2)),
                TEXTE2,
            );
            ui.add_space(8.0);
        }
        ui.add_space(2.0);
        let note = if firefox {
            t(
                l,
                "Temporary add-on: Firefox removes it when it closes, until Mozilla signs the extension.",
            )
        } else {
            t(
                l,
                "This folder is updated with fluxdl: after an update, click Reload on the extension's card.",
            )
        };
        ui.label(egui::RichText::new(note).font(regulier(12.5)).color(TEXTE3));
        if let Some(e) = &g.erreur {
            ui.label(egui::RichText::new(e).font(regulier(12.5)).color(DANGER));
        }
        let fermer = boutons(ui, t(l, "Close"), "", false).is_some();
        if !fermer {
            self.modale = Some(Modale::Guide(g));
        }
        fermer
    }

    /// Bulle du premier lancement : proposer l'extension, une seule fois.
    pub(super) fn bulle_extension(&mut self, ctx: &egui::Context) {
        let maj_visible = matches!(
            self.maj.etat(),
            maj::Etat::Dispo(_)
                | maj::Etat::Telechargement(..)
                | maj::Etat::Pret(..)
                | maj::Etat::Echec(_)
        );
        if self.settings.extension_asked || self.modale.is_some() || maj_visible {
            return;
        }
        let l = self.settings.lang;
        let ecran = ctx.content_rect();
        let taille = vec2(380.0, 112.0);
        let pos = ecran.max - taille - vec2(28.0, 26.0);
        egui::Area::new(egui::Id::unique("bulle-extension"))
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
                orbe(p, c, 17.0, GEL_BLEU);
                icone_relief(p, Rect::from_center_size(c, Vec2::splat(16.0)), Icone::Lien);
                let x = r.left() + 62.0;
                let g = ligne(
                    ui,
                    t(l, "Add fluxdl to your browser"),
                    demi(14.5),
                    TITRE,
                    r.right() - x - 14.0,
                );
                p.galley(pos2(x, r.top() + 27.0 - g.size().y / 2.0), g, TITRE);
                let g = ligne(
                    ui,
                    t(l, "Right-click menu, capture of browser downloads."),
                    regulier(12.5),
                    TEXTE2,
                    r.right() - x - 14.0,
                );
                p.galley(pos2(x, r.top() + 47.0 - g.size().y / 2.0), g, TEXTE2);
                let bp = Rect::from_min_size(
                    pos2(r.right() - 16.0 - 130.0, r.bottom() - 44.0),
                    vec2(130.0, 30.0),
                );
                let bl = Rect::from_min_size(pos2(bp.left() - 98.0, bp.top()), vec2(90.0, 30.0));
                let plus_tard = a_la_place(ui, bl, |ui| {
                    bouton_secondaire(ui, t(l, "Later"), None, bl.size())
                });
                let installer = a_la_place(ui, bp, |ui| {
                    bouton_gel(
                        ui,
                        t(l, "Install"),
                        Some(Icone::Telecharger),
                        bp.size(),
                        GEL_BLEU,
                    )
                });
                if plus_tard.clicked() {
                    self.settings.extension_asked = true;
                }
                if installer.clicked() {
                    self.ouvrir_guide();
                }
            });
    }

    /// Captures : guide avec des navigateurs fictifs (Windows), Chrome ou Firefox.
    pub(super) fn demo_guide(&mut self, firefox: bool) {
        let nav = |nom, exe: &str, page, firefox| Navigateur {
            nom,
            exe: exe.into(),
            page,
            firefox,
        };
        let mut g = Guide::new(
            vec![
                nav("Chrome", "chrome.exe", "chrome://extensions", false),
                nav("Edge", "msedge.exe", "edge://extensions", false),
                nav("Brave", "brave.exe", "brave://extensions", false),
                nav(
                    "Firefox",
                    "firefox.exe",
                    "about:debugging#/runtime/this-firefox",
                    true,
                ),
            ],
            r"C:\Users\Camille\AppData\Local\fluxdl\extension".into(),
            r"C:\Users\Camille\Downloads\fluxdl-firefox-0.2.3.zip".into(),
            None,
        );
        g.choix = if firefox { 3 } else { 0 };
        g.ouvert = !firefox;
        self.modale = Some(Modale::Guide(g));
    }
}

/// Fenêtre de navigateur miniature (fond blanc, barre d'adresse) ; renvoie la zone de la page.
fn fenetre(p: &egui::Painter, r: Rect, adresse: &str) -> Rect {
    p.add(lueur(r.translate(vec2(0.0, 2.0)), nuit(40), 6.0));
    p.rect_filled(r, 7.0, rgb(0xf4, 0xf7, 0xfb));
    let barre = Rect::from_min_size(r.min, vec2(r.width(), 18.0));
    p.rect_filled(
        barre,
        CornerRadius {
            nw: 7,
            ne: 7,
            sw: 0,
            se: 0,
        },
        rgb(0xdd, 0xe5, 0xef),
    );
    let champ = Rect::from_min_max(barre.min + vec2(8.0, 4.0), barre.max - vec2(8.0, 4.0));
    p.rect_filled(champ, 5.0, Color32::WHITE);
    p.text(
        champ.left_center() + vec2(5.0, 0.0),
        Align2::LEFT_CENTER,
        adresse,
        regulier(7.5),
        TEXTE3,
    );
    p.rect_stroke(r, 7.0, Stroke::new(1.0, nuit(60)), StrokeKind::Inside);
    Rect::from_min_max(pos2(r.left(), barre.bottom()), r.max)
}

/// Contour orange autour de l'élément à cliquer, et le pointeur de la souris.
fn repere(p: &egui::Painter, r: Rect, rayon: f32) {
    p.add(lueur(r.expand(2.0), REPERE.gamma_multiply(0.45), 5.0));
    p.rect_stroke(
        r.expand(2.0),
        rayon + 2.0,
        Stroke::new(1.8, REPERE),
        StrokeKind::Outside,
    );
    let o = r.right_bottom() - vec2(6.0, 5.0);
    let pts = [
        (0.0, 0.0),
        (0.0, 13.0),
        (3.2, 10.0),
        (5.6, 15.0),
        (7.6, 14.0),
        (5.3, 9.2),
        (9.5, 9.2),
    ];
    let v: Vec<Pos2> = pts.iter().map(|(x, y)| o + vec2(*x, *y)).collect();
    p.add(Shape::convex_polygon(
        v.clone(),
        Color32::WHITE,
        Stroke::NONE,
    ));
    p.add(Shape::closed_line(v, Stroke::new(1.0, TEXTE)));
}

/// Barres grises : lignes de texte factices.
fn lignes(p: &egui::Painter, x: f32, y: f32, largeurs: &[f32]) {
    for (i, w) in largeurs.iter().enumerate() {
        p.rect_filled(
            Rect::from_min_size(pos2(x, y + i as f32 * 7.0), vec2(*w, 3.0)),
            1.5,
            rgb(0xd5, 0xdc, 0xe6),
        );
    }
}

/// Bouton factice (texte minuscule) à la taille de son texte, au plus `max` de large, posé
/// à partir de `coin` (coin haut gauche, ou haut droit si `droite`).
fn bouton_mock(
    p: &egui::Painter,
    coin: Pos2,
    droite: bool,
    s: &str,
    plein: bool,
    max: f32,
) -> Rect {
    let (fond, texte) = if plein {
        (rgb(0x1a, 0x73, 0xe8), Color32::WHITE)
    } else {
        (Color32::WHITE, rgb(0x1a, 0x5f, 0xc8))
    };
    let g = p.layout_no_wrap(s.into(), demi(7.0), texte);
    let w = (g.size().x + 12.0).clamp(22.0, max);
    let x = if droite { coin.x - w } else { coin.x };
    let r = Rect::from_min_size(pos2(x, coin.y), vec2(w, 13.0));
    p.rect_filled(r, 4.0, fond);
    if !plein {
        p.rect_stroke(
            r,
            4.0,
            Stroke::new(1.0, rgb(0xa8, 0xc3, 0xe8)),
            StrokeKind::Inside,
        );
    }
    let pos = pos2(
        (r.center().x - g.size().x / 2.0).max(r.left() + 6.0),
        r.center().y - g.size().y / 2.0,
    );
    p.with_clip_rect(r.shrink2(vec2(4.0, 0.0)))
        .galley(pos, g, texte);
    r
}

/// Illustration de l'étape `i` (0 à 2), d'après la mise en page réelle des pages.
fn dessin(p: &egui::Painter, r: Rect, i: usize, firefox: bool, l: Lang) {
    if firefox {
        match i {
            0 | 1 => {
                let page = fenetre(p, r, "about:debugging");
                // Colonne de gauche : « Ce Firefox » ; page : titre et bouton du module.
                let col = Rect::from_min_size(page.min, vec2(50.0, page.height()));
                p.rect_filled(
                    col,
                    CornerRadius {
                        sw: 7,
                        ..Default::default()
                    },
                    rgb(0xe9, 0xee, 0xf5),
                );
                let item = Rect::from_min_size(col.min + vec2(5.0, 8.0), vec2(40.0, 12.0));
                p.rect_filled(
                    item,
                    3.0,
                    if i == 0 {
                        rgb(0xd6, 0xe6, 0xfb)
                    } else {
                        Color32::TRANSPARENT
                    },
                );
                p.text(
                    item.left_center() + vec2(3.0, 0.0),
                    Align2::LEFT_CENTER,
                    t(l, "This Firefox"),
                    demi(7.0),
                    TEXTE,
                );
                lignes(p, col.left() + 8.0, item.bottom() + 8.0, &[30.0, 24.0]);
                let x = col.right() + 8.0;
                p.text(
                    pos2(x, page.top() + 12.0),
                    Align2::LEFT_CENTER,
                    t(l, "This Firefox"),
                    demi(9.0),
                    TEXTE,
                );
                let b = bouton_mock(
                    p,
                    pos2(x, page.top() + 22.0),
                    false,
                    t(l, "Load Temporary Add-on…"),
                    false,
                    page.right() - x - 6.0,
                );
                lignes(p, x, b.bottom() + 9.0, &[90.0, 70.0, 80.0]);
                repere(p, if i == 0 { item } else { b }, 3.0);
            }
            _ => choix_fichier(p, r, true, l),
        }
    } else {
        match i {
            0 | 1 => {
                let page = fenetre(p, r, "chrome://extensions");
                p.text(
                    pos2(page.left() + 8.0, page.top() + 11.0),
                    Align2::LEFT_CENTER,
                    "Extensions",
                    demi(9.0),
                    TEXTE,
                );
                // Interrupteur « Mode développeur » en haut à droite.
                let inter = Rect::from_min_size(
                    pos2(page.right() - 24.0, page.top() + 6.0),
                    vec2(16.0, 9.0),
                );
                let on = i == 1;
                p.rect_filled(
                    inter,
                    4.5,
                    if on {
                        rgb(0x1a, 0x73, 0xe8)
                    } else {
                        rgb(0xbd, 0xc6, 0xd2)
                    },
                );
                let cx = if on {
                    inter.right() - 4.5
                } else {
                    inter.left() + 4.5
                };
                p.circle_filled(pos2(cx, inter.center().y), 3.6, Color32::WHITE);
                p.text(
                    inter.left_center() - vec2(4.0, 0.0),
                    Align2::RIGHT_CENTER,
                    t(l, "Developer mode"),
                    regulier(7.0),
                    TEXTE2,
                );
                if on {
                    let b = bouton_mock(
                        p,
                        pos2(page.left() + 8.0, page.top() + 22.0),
                        false,
                        t(l, "Load unpacked"),
                        false,
                        page.width() - 16.0,
                    );
                    // Boutons voisins (Empaqueter, Mettre à jour), s'il reste de la place.
                    let mut x = b.right() + 5.0;
                    while x + 22.0 <= page.right() - 8.0 {
                        bouton_mock(p, pos2(x, b.top()), false, "", false, 22.0);
                        x += 27.0;
                    }
                    repere(p, b, 4.0);
                } else {
                    repere(p, inter, 4.5);
                }
                // Cartes d'extensions factices.
                let y = page.top() + if on { 42.0 } else { 26.0 };
                for k in 0..2 {
                    let c = Rect::from_min_size(
                        pos2(page.left() + 8.0 + k as f32 * 76.0, y),
                        vec2(70.0, page.bottom() - y - 6.0),
                    );
                    p.rect_filled(c, 4.0, Color32::WHITE);
                    p.rect_stroke(
                        c,
                        4.0,
                        Stroke::new(1.0, rgb(0xdd, 0xe3, 0xeb)),
                        StrokeKind::Inside,
                    );
                    lignes(p, c.left() + 6.0, c.top() + 6.0, &[40.0, 52.0]);
                }
            }
            _ => choix_fichier(p, r, false, l),
        }
    }
}

/// Sélecteur de fichier/dossier Windows : chemin collé, bouton de validation repéré.
fn choix_fichier(p: &egui::Painter, r: Rect, firefox: bool, l: Lang) {
    p.add(lueur(r.translate(vec2(0.0, 2.0)), nuit(40), 6.0));
    p.rect_filled(r, 6.0, rgb(0xf7, 0xf9, 0xfc));
    let titre = Rect::from_min_size(r.min, vec2(r.width(), 14.0));
    p.rect_filled(
        titre,
        CornerRadius {
            nw: 6,
            ne: 6,
            sw: 0,
            se: 0,
        },
        Color32::WHITE,
    );
    p.text(
        titre.left_center() + vec2(6.0, 0.0),
        Align2::LEFT_CENTER,
        if firefox {
            t(l, "Open")
        } else {
            t(l, "Select Folder")
        },
        regulier(7.0),
        TEXTE2,
    );
    p.rect_stroke(r, 6.0, Stroke::new(1.0, nuit(60)), StrokeKind::Inside);
    // Liste de fichiers factice, le fichier ou dossier visé surligné.
    let liste = Rect::from_min_max(
        r.min + vec2(6.0, 19.0),
        pos2(r.right() - 6.0, r.top() + 50.0),
    );
    p.rect_filled(liste, 3.0, Color32::WHITE);
    let sel = Rect::from_min_size(liste.min + vec2(3.0, 3.0), vec2(liste.width() - 6.0, 9.0));
    p.rect_filled(sel, 2.0, rgb(0xcc, 0xe4, 0xff));
    let nom = if firefox {
        "fluxdl-firefox.zip"
    } else {
        "extension"
    };
    p.rect_filled(
        Rect::from_min_size(sel.min + vec2(3.0, 2.0), vec2(6.0, 5.0)),
        1.0,
        rgb(0xf2, 0xc1, 0x4e),
    );
    p.text(
        sel.left_center() + vec2(12.0, 0.0),
        Align2::LEFT_CENTER,
        nom,
        regulier(7.0),
        TEXTE,
    );
    lignes(p, liste.left() + 15.0, sel.bottom() + 5.0, &[50.0, 38.0]);
    // Champ du nom avec le chemin collé, puis le bouton.
    let y = r.bottom() - 18.0;
    let champ = Rect::from_min_size(pos2(r.left() + 6.0, y - 16.0), vec2(r.width() - 12.0, 11.0));
    p.rect_filled(champ, 2.0, Color32::WHITE);
    p.rect_stroke(
        champ,
        2.0,
        Stroke::new(1.0, rgb(0x7a, 0x9c, 0xc6)),
        StrokeKind::Inside,
    );
    p.text(
        champ.left_center() + vec2(3.0, 0.0),
        Align2::LEFT_CENTER,
        if firefox {
            r"C:\…\Downloads\fluxdl-firefox.zip"
        } else {
            r"C:\…\fluxdl\extension"
        },
        regulier(6.5),
        TEXTE,
    );
    let b = bouton_mock(
        p,
        pos2(r.right() - 6.0, y - 2.0),
        true,
        if firefox {
            t(l, "Open")
        } else {
            t(l, "Select Folder")
        },
        true,
        r.width() - 12.0,
    );
    repere(p, b, 4.0);
}
