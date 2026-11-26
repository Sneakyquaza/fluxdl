//! Style (Inter OFL, jetons de couleur, palette en dégradé) et dessins : icônes vectorielles,
//! boutons, carte du fichier, une barre par connexion, courbe de débit.

use dlcore::engine::Live;
use eframe::egui::{
    self, epaint::RectShape, pos2, vec2, Align2, Color32, CornerRadius, FontFamily, FontId, Margin,
    Mesh, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, TextureId, Ui, Vec2,
};
use std::sync::{Arc, OnceLock};

pub const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// Fond de la zone principale, barre latérale, panneau de détail.
pub const FOND: Color32 = rgb(0x0c, 0x0e, 0x13);
pub const LATERAL: Color32 = rgb(0x10, 0x12, 0x18);
/// Surfaces superposées : carte, carte survolée / champ, creux des jauges.
pub const SURFACE: Color32 = rgb(0x15, 0x18, 0x1f);
pub const SURFACE_HAUTE: Color32 = rgb(0x1c, 0x20, 0x29);
pub const CREUX: Color32 = rgb(0x0a, 0x0b, 0x10);
pub const PISTE: Color32 = rgb(0x20, 0x24, 0x2e);
pub const BORD: Color32 = rgb(0x22, 0x26, 0x30);
pub const BORD_FORT: Color32 = rgb(0x30, 0x36, 0x43);
pub const TEXTE: Color32 = rgb(0xec, 0xee, 0xf3);
pub const TEXTE2: Color32 = rgb(0x9a, 0xa1, 0xb0);
pub const TEXTE3: Color32 = rgb(0x6e, 0x75, 0x85);
pub const ACCENT: Color32 = rgb(0x70, 0x7c, 0xff);
pub const SUCCES: Color32 = rgb(0x3c, 0xd1, 0x95);
pub const ATTENTION: Color32 = rgb(0xf2, 0xb2, 0x4c);
pub const DANGER: Color32 = rgb(0xf2, 0x5f, 0x6b);

/// Palette des connexions et de l'accent : aqua → bleu → indigo → violet → orchidée.
const ARRETS: [(f32, Color32); 5] = [
    (0.0, rgb(0x2e, 0xd3, 0xc6)),
    (0.3, rgb(0x4c, 0x9e, 0xff)),
    (0.55, rgb(0x6e, 0x7b, 0xff)),
    (0.8, rgb(0x9a, 0x64, 0xff)),
    (1.0, rgb(0xd4, 0x6a, 0xe0)),
];
/// Plage du dégradé d'accent (boutons, barres de progression).
pub const ACCENT_UV: (f32, f32) = (0.42, 0.86);

pub fn teinte(t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let j = ARRETS.iter().position(|a| a.0 >= t).unwrap_or(4).max(1);
    let ((t0, a), (t1, b)) = (ARRETS[j - 1], ARRETS[j]);
    mix(a, b, (t - t0) / (t1 - t0))
}

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(
        m(a.r(), b.r()),
        m(a.g(), b.g()),
        m(a.b(), b.b()),
        m(a.a(), b.a()),
    )
}

/// Position d'une connexion dans la palette (0..1).
pub fn rang(w: usize, n: usize) -> f32 {
    if n <= 1 {
        0.5
    } else {
        w as f32 / (n - 1) as f32
    }
}

pub fn couleur_connexion(w: usize, n: usize) -> Color32 {
    teinte(rang(w, n))
}

static DEGRADE: OnceLock<TextureId> = OnceLock::new();

pub fn regulier(t: f32) -> FontId {
    FontId::proportional(t)
}
pub fn moyen(t: f32) -> FontId {
    FontId::new(t, FontFamily::Name("moyen".into()))
}
pub fn demi(t: f32) -> FontId {
    FontId::new(t, FontFamily::Name("demi".into()))
}
pub fn affiche(t: f32) -> FontId {
    FontId::new(t, FontFamily::Name("affiche".into()))
}

/// Polices, visuels egui et texture de la palette. Renvoie la texture (à garder en vie).
pub fn installer_style(ctx: &egui::Context) -> egui::TextureHandle {
    let mut polices = egui::FontDefinitions::empty();
    // Inter 4.1, sous-ensemble latin, chiffres tabulaires figés (débits et tailles alignés).
    for (nom, octets) in [
        (
            "inter",
            &include_bytes!("../../assets/fonts/Inter-Regular.ttf")[..],
        ),
        (
            "inter-moyen",
            &include_bytes!("../../assets/fonts/Inter-Medium.ttf")[..],
        ),
        (
            "inter-demi",
            &include_bytes!("../../assets/fonts/Inter-SemiBold.ttf")[..],
        ),
        (
            "inter-affiche",
            &include_bytes!("../../assets/fonts/InterDisplay-SemiBold.ttf")[..],
        ),
    ] {
        polices
            .font_data
            .insert(nom.into(), Arc::new(egui::FontData::from_static(octets)));
    }
    for (famille, nom) in [
        (FontFamily::Proportional, "inter"),
        (FontFamily::Monospace, "inter"),
        (FontFamily::Name("moyen".into()), "inter-moyen"),
        (FontFamily::Name("demi".into()), "inter-demi"),
        (FontFamily::Name("affiche".into()), "inter-affiche"),
    ] {
        polices
            .families
            .insert(famille, vec![nom.to_string(), "inter".to_string()]);
    }
    ctx.set_fonts(polices);
    let mut style = (*ctx.global_style()).clone();
    style.text_styles = [
        (egui::TextStyle::Small, regulier(12.0)),
        (egui::TextStyle::Body, regulier(14.0)),
        (egui::TextStyle::Button, moyen(13.5)),
        (egui::TextStyle::Heading, affiche(24.0)),
        (egui::TextStyle::Monospace, regulier(13.0)),
    ]
    .into();
    let s = &mut style.spacing;
    s.item_spacing = vec2(8.0, 8.0);
    s.button_padding = vec2(12.0, 7.0);
    s.interact_size = vec2(32.0, 32.0);
    s.slider_width = 240.0;
    s.slider_rail_height = 4.0;
    s.scroll = egui::style::ScrollStyle::floating();
    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.override_text_color = Some(TEXTE);
    v.panel_fill = FOND;
    v.window_fill = SURFACE_HAUTE;
    v.window_stroke = Stroke::new(1.0, BORD_FORT);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(10);
    v.popup_shadow = egui::Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(110),
    };
    v.extreme_bg_color = CREUX;
    v.faint_bg_color = SURFACE;
    v.selection.bg_fill = rgb(0x4d, 0x5a, 0xe0);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.text_cursor.stroke = Stroke::new(1.5, ACCENT);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    for (w, fond, bord, texte) in [
        (&mut v.widgets.inactive, SURFACE_HAUTE, BORD, TEXTE2),
        (
            &mut v.widgets.hovered,
            rgb(0x24, 0x29, 0x34),
            BORD_FORT,
            TEXTE,
        ),
        (
            &mut v.widgets.active,
            rgb(0x2a, 0x30, 0x3d),
            BORD_FORT,
            TEXTE,
        ),
        (&mut v.widgets.open, rgb(0x24, 0x29, 0x34), BORD_FORT, TEXTE),
        (&mut v.widgets.noninteractive, SURFACE, BORD, TEXTE2),
    ] {
        w.bg_fill = fond;
        w.weak_bg_fill = fond;
        w.bg_stroke = Stroke::new(1.0, bord);
        w.corner_radius = CornerRadius::same(8);
        w.expansion = 0.0;
        w.fg_stroke = Stroke::new(1.5, texte);
    }
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_global_style(style);
    let img = egui::ColorImage::new(
        [256, 1],
        (0..256).map(|i| teinte(i as f32 / 255.0)).collect(),
    );
    let tex = ctx.load_texture("palette", img, egui::TextureOptions::LINEAR);
    let _ = DEGRADE.set(tex.id());
    tex
}

/// Forme arrondie remplie par une portion `t0..t1` de la palette (dégradé horizontal).
pub fn degrade(r: Rect, rayon: impl Into<CornerRadius>, t0: f32, t1: f32, voile: Color32) -> Shape {
    let Some(tex) = DEGRADE.get() else {
        return Shape::rect_filled(r, rayon, teinte((t0 + t1) / 2.0));
    };
    let uv = Rect::from_min_max(pos2(t0, 0.5), pos2(t1, 0.5));
    RectShape::filled(r, rayon, voile)
        .with_texture(*tex, uv)
        .into()
}

/// Cadre de carte : surface, liseré discret, ombre douce.
pub fn carte() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORD))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(20))
        .shadow(egui::Shadow {
            offset: [0, 6],
            blur: 20,
            spread: 0,
            color: Color32::from_black_alpha(60),
        })
}

/// Galerie d'une ligne tronquée par « … » à `largeur`.
pub fn ligne(
    ui: &Ui,
    texte: impl Into<String>,
    police: FontId,
    c: Color32,
    largeur: f32,
) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(texte.into(), police, c);
    job.wrap = egui::text::TextWrapping::truncate_at_width(largeur.max(8.0));
    ui.painter().layout_job(job)
}

/// Texte posé par une ancre (gauche/droite, centré verticalement) ; renvoie sa largeur.
pub fn texte(ui: &Ui, pos: Pos2, ancre: Align2, t: &str, police: FontId, c: Color32) -> f32 {
    ui.painter().text(pos, ancre, t, police, c).width()
}

#[derive(Clone, Copy, PartialEq)]
pub enum Icone {
    Telecharger,
    Comparatif,
    Reglages,
    Pause,
    Lecture,
    Fermer,
    Dossier,
    Coller,
    Lien,
    Eclair,
}

fn trait_(p: &egui::Painter, pts: &[Pos2], s: Stroke) {
    p.add(Shape::line(pts.to_vec(), s));
    // Bouts ronds.
    for q in [pts[0], pts[pts.len() - 1]] {
        p.circle_filled(q, s.width / 2.0, s.color);
    }
}

/// Icône au trait de 1,6 px dans le carré `r` (16 px de référence).
pub fn icone(p: &egui::Painter, r: Rect, i: Icone, c: Color32) {
    let k = r.width() / 16.0;
    let o = r.min;
    let q = |x: f32, y: f32| o + vec2(x * k, y * k);
    let s = Stroke::new(1.6 * k, c);
    match i {
        Icone::Telecharger => {
            trait_(p, &[q(8.0, 2.0), q(8.0, 10.0)], s);
            trait_(p, &[q(4.5, 6.8), q(8.0, 10.3), q(11.5, 6.8)], s);
            trait_(p, &[q(2.5, 13.5), q(13.5, 13.5)], s);
        }
        Icone::Comparatif => {
            trait_(p, &[q(3.5, 13.0), q(3.5, 9.0)], s);
            trait_(p, &[q(8.0, 13.0), q(8.0, 6.0)], s);
            trait_(p, &[q(12.5, 13.0), q(12.5, 3.0)], s);
        }
        Icone::Reglages => {
            for (y, x) in [(4.0, 10.5), (8.0, 5.5), (12.0, 9.0)] {
                trait_(p, &[q(2.0, y), q(14.0, y)], Stroke::new(1.3 * k, c));
                p.circle_filled(q(x, y), 2.2 * k, LATERAL);
                p.circle_stroke(q(x, y), 1.9 * k, Stroke::new(1.4 * k, c));
            }
        }
        Icone::Pause => {
            trait_(p, &[q(5.0, 3.0), q(5.0, 13.0)], Stroke::new(2.4 * k, c));
            trait_(p, &[q(11.0, 3.0), q(11.0, 13.0)], Stroke::new(2.4 * k, c));
        }
        Icone::Lecture => {
            p.add(Shape::convex_polygon(
                vec![q(5.0, 3.0), q(13.0, 8.0), q(5.0, 13.0)],
                c,
                Stroke::new(1.0 * k, c),
            ));
        }
        Icone::Fermer => {
            trait_(p, &[q(4.0, 4.0), q(12.0, 12.0)], s);
            trait_(p, &[q(12.0, 4.0), q(4.0, 12.0)], s);
        }
        Icone::Dossier => {
            let pts = vec![
                q(2.0, 4.5),
                q(2.0, 12.5),
                q(14.0, 12.5),
                q(14.0, 6.0),
                q(8.0, 6.0),
                q(6.5, 3.5),
                q(2.5, 3.5),
                q(2.0, 4.5),
            ];
            p.add(Shape::closed_line(pts, s));
        }
        Icone::Coller => {
            p.rect_stroke(
                Rect::from_min_max(q(3.0, 3.0), q(13.0, 14.5)),
                2.0 * k,
                s,
                StrokeKind::Middle,
            );
            p.rect_filled(Rect::from_min_max(q(5.5, 1.5), q(10.5, 4.5)), 1.5 * k, c);
            trait_(p, &[q(5.5, 8.0), q(10.5, 8.0)], Stroke::new(1.3 * k, c));
            trait_(p, &[q(5.5, 11.0), q(9.0, 11.0)], Stroke::new(1.3 * k, c));
        }
        Icone::Lien => {
            for (cx, cy) in [(5.8, 10.2), (10.2, 5.8)] {
                let m = RectShape::stroke(
                    Rect::from_center_size(q(cx, cy), vec2(9.0 * k, 5.0 * k)),
                    2.5 * k,
                    Stroke::new(1.5 * k, c),
                    StrokeKind::Middle,
                )
                .with_angle(-std::f32::consts::FRAC_PI_4);
                p.add(m);
            }
        }
        Icone::Eclair => {
            p.add(Shape::convex_polygon(
                vec![q(9.5, 1.5), q(3.5, 9.0), q(8.0, 9.0)],
                c,
                Stroke::NONE,
            ));
            p.add(Shape::convex_polygon(
                vec![q(8.0, 7.0), q(12.5, 7.0), q(6.5, 14.5)],
                c,
                Stroke::NONE,
            ));
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum TypeFichier {
    Archive,
    Video,
    Disque,
    Document,
    Autre,
}

impl TypeFichier {
    pub fn de(nom: &str) -> Self {
        let n = nom.to_ascii_lowercase();
        let ext = n.rsplit('.').next().unwrap_or("");
        match ext {
            "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "xz" | "bz2" | "zst" | "lz4" | "cab" => {
                TypeFichier::Archive
            }
            "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "ts" | "flv" | "wmv" => {
                TypeFichier::Video
            }
            "iso" | "img" | "dmg" | "vhd" | "vhdx" | "qcow2" | "vmdk" => TypeFichier::Disque,
            "pdf" | "doc" | "docx" | "odt" | "txt" | "md" | "epub" | "rtf" | "xls" | "xlsx"
            | "ppt" | "pptx" | "csv" => TypeFichier::Document,
            _ => TypeFichier::Autre,
        }
    }

    fn couleur(self) -> Color32 {
        match self {
            TypeFichier::Archive => rgb(0xe8, 0xa8, 0x5a),
            TypeFichier::Video => rgb(0xf0, 0x6e, 0x95),
            TypeFichier::Disque => rgb(0x3c, 0xcb, 0xc2),
            TypeFichier::Document => rgb(0x6c, 0x9c, 0xff),
            TypeFichier::Autre => rgb(0xa5, 0x8c, 0xff),
        }
    }
}

/// Tuile de type de fichier : carré arrondi teinté, pictogramme au trait.
pub fn tuile_fichier(p: &egui::Painter, r: Rect, t: TypeFichier) {
    let c = t.couleur();
    p.rect_filled(r, r.width() * 0.26, c.gamma_multiply(0.13));
    p.rect_stroke(
        r,
        r.width() * 0.26,
        Stroke::new(1.0, c.gamma_multiply(0.22)),
        StrokeKind::Inside,
    );
    let g = Rect::from_center_size(r.center(), Vec2::splat(r.width() * 0.5));
    let k = g.width() / 16.0;
    let q = |x: f32, y: f32| g.min + vec2(x * k, y * k);
    let s = Stroke::new(1.5 * k.max(0.9), c);
    let page = |p: &egui::Painter| {
        let pts = vec![
            q(3.5, 1.0),
            q(9.5, 1.0),
            q(13.0, 4.5),
            q(13.0, 15.0),
            q(3.5, 15.0),
            q(3.5, 1.0),
        ];
        p.add(Shape::closed_line(pts, s));
        p.add(Shape::line(vec![q(9.5, 1.0), q(9.5, 4.5), q(13.0, 4.5)], s));
    };
    match t {
        TypeFichier::Archive => {
            p.rect_stroke(
                Rect::from_min_max(q(2.5, 1.5), q(13.5, 14.5)),
                2.0 * k,
                s,
                StrokeKind::Middle,
            );
            for y in [3.5, 6.0, 8.5] {
                p.line_segment([q(7.0, y), q(8.0, y)], s);
                p.line_segment([q(8.0, y + 1.25), q(9.0, y + 1.25)], s);
            }
            p.rect_filled(Rect::from_min_max(q(6.5, 11.0), q(9.5, 13.0)), 1.0 * k, c);
        }
        TypeFichier::Video => {
            p.rect_stroke(
                Rect::from_min_max(q(1.5, 3.0), q(14.5, 13.0)),
                2.5 * k,
                s,
                StrokeKind::Middle,
            );
            p.add(Shape::convex_polygon(
                vec![q(6.5, 5.5), q(11.0, 8.0), q(6.5, 10.5)],
                c,
                Stroke::NONE,
            ));
        }
        TypeFichier::Disque => {
            p.circle_stroke(q(8.0, 8.0), 6.5 * k, s);
            p.circle_stroke(q(8.0, 8.0), 2.0 * k, s);
            p.add(Shape::line(
                (0..8)
                    .map(|i| {
                        let a = -2.6 + i as f32 * 0.12;
                        q(8.0 + 4.3 * a.cos(), 8.0 + 4.3 * a.sin())
                    })
                    .collect(),
                Stroke::new(1.2 * k, c.gamma_multiply(0.7)),
            ));
        }
        TypeFichier::Document => {
            page(p);
            for (y, l) in [(8.0, 6.0), (10.5, 6.0), (13.0, 4.0)] {
                p.line_segment(
                    [q(5.8, y - 0.5), q(5.8 + l, y - 0.5)],
                    Stroke::new(1.2 * k, c),
                );
            }
        }
        TypeFichier::Autre => page(p),
    }
}

/// Pastille d'état : point + texte sur fond teinté. Renvoie le rectangle occupé.
pub fn pastille(ui: &Ui, gauche_centre: Pos2, t: &str, c: Color32) -> Rect {
    let g = ui.painter().layout_no_wrap(t.to_string(), moyen(11.5), c);
    let r = Rect::from_min_size(
        pos2(gauche_centre.x, gauche_centre.y - 10.0),
        vec2(g.size().x + 28.0, 20.0),
    );
    let p = ui.painter();
    p.rect_filled(r, 10.0, c.gamma_multiply(0.13));
    p.circle_filled(pos2(r.left() + 11.0, r.center().y), 3.0, c);
    p.galley(pos2(r.left() + 19.0, r.center().y - g.size().y / 2.0), g, c);
    r
}

/// Fond de survol animé (0 → 1) d'un élément interactif.
pub fn survol(ui: &Ui, r: &Response) -> f32 {
    ui.ctx()
        .animate_bool_with_time(r.id.with("survol"), r.hovered(), 0.12)
}

/// Bouton à icône à un emplacement donné (au-dessus d'une carte cliquable).
pub fn bouton_icone_ici(ui: &Ui, r: Rect, id: egui::Id, i: Icone, aide: &str) -> Response {
    let rep = ui.interact(r, id, Sense::click());
    let h = survol(ui, &rep);
    let p = ui.painter();
    if h > 0.0 {
        p.rect_filled(r, 8.0, PISTE.gamma_multiply(h));
        p.rect_stroke(
            r,
            8.0,
            Stroke::new(1.0, BORD_FORT.gamma_multiply(h)),
            StrokeKind::Inside,
        );
    }
    let c = if rep.is_pointer_button_down_on() {
        TEXTE2
    } else {
        mix(TEXTE3, TEXTE, 0.35 + 0.65 * h)
    };
    icone(
        p,
        Rect::from_center_size(r.center(), Vec2::splat(15.0)),
        i,
        c,
    );
    anneau_focus(ui, &rep, r, 8.0);
    rep.on_hover_text(aide)
}

/// Champ de saisie d'une ligne : creux arrondi, icône facultative, liseré d'accent au focus.
pub fn champ(
    ui: &mut Ui,
    r: Rect,
    texte: &mut String,
    indice: &str,
    i: Option<Icone>,
    police: FontId,
) -> Response {
    let p = ui.painter().clone();
    p.rect_filled(r, 10.0, CREUX);
    let g = if let Some(i) = i {
        icone(
            &p,
            Rect::from_min_size(pos2(r.left() + 14.0, r.center().y - 8.0), Vec2::splat(16.0)),
            i,
            TEXTE3,
        );
        40.0
    } else {
        12.0
    };
    let zone = Rect::from_x_y_ranges(r.left() + g..=r.right() - 10.0, r.y_range());
    let h = ui.ctx().fonts_mut(|f| f.row_height(&police));
    let marge = ((r.height() - h) / 2.0 - 0.5).max(0.0) as i8;
    let rep = ui.put(
        zone,
        egui::TextEdit::singleline(texte)
            .hint_text(egui::RichText::new(indice).color(TEXTE3))
            .font(police)
            .frame(egui::Frame::NONE.inner_margin(Margin::symmetric(0, marge)))
            .margin(Margin::symmetric(0, marge))
            .desired_width(zone.width()),
    );
    let c = if rep.has_focus() {
        ACCENT.gamma_multiply(0.8)
    } else if rep.hovered() {
        BORD_FORT
    } else {
        BORD
    };
    p.rect_stroke(r, 10.0, Stroke::new(1.0, c), StrokeKind::Inside);
    rep
}

/// Bouton principal : dégradé d'accent, halo doux, texte blanc.
pub fn bouton_principal(ui: &mut Ui, t: &str, i: Option<Icone>, taille: Vec2) -> Response {
    let (r, rep) = ui.allocate_exact_size(taille, Sense::click());
    let h = survol(ui, &rep);
    let p = ui.painter();
    p.add(
        egui::Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: ACCENT.gamma_multiply(0.22 + 0.18 * h),
        }
        .as_shape(r, 10),
    );
    let voile = if rep.is_pointer_button_down_on() {
        Color32::from_gray(205)
    } else {
        mix(Color32::from_gray(232), Color32::WHITE, h)
    };
    p.add(degrade(r, 10, ACCENT_UV.0, ACCENT_UV.1, voile));
    // Liseré clair en haut : relief.
    p.rect_stroke(
        r,
        10.0,
        Stroke::new(1.0, Color32::from_white_alpha(28)),
        StrokeKind::Inside,
    );
    let g = p.layout_no_wrap(t.to_string(), demi(14.0), Color32::WHITE);
    let li = if i.is_some() { 22.0 } else { 0.0 };
    let x = r.center().x - (g.size().x + li) / 2.0;
    if let Some(i) = i {
        icone(
            p,
            Rect::from_min_size(pos2(x, r.center().y - 8.0), Vec2::splat(16.0)),
            i,
            Color32::WHITE,
        );
    }
    p.galley(
        pos2(x + li, r.center().y - g.size().y / 2.0),
        g,
        Color32::WHITE,
    );
    anneau_focus(ui, &rep, r, 10.0);
    rep
}

/// Bouton secondaire : surface, liseré, icône facultative.
pub fn bouton_secondaire(ui: &mut Ui, t: &str, i: Option<Icone>, taille: Vec2) -> Response {
    let (r, rep) = ui.allocate_exact_size(taille, Sense::click());
    let h = survol(ui, &rep);
    let p = ui.painter();
    p.rect_filled(r, 10.0, mix(SURFACE_HAUTE, rgb(0x26, 0x2b, 0x37), h));
    p.rect_stroke(
        r,
        10.0,
        Stroke::new(1.0, mix(BORD, BORD_FORT, h)),
        StrokeKind::Inside,
    );
    let c = mix(TEXTE2, TEXTE, 0.5 + 0.5 * h);
    let g = p.layout_no_wrap(t.to_string(), moyen(13.5), c);
    let li = match (i.is_some(), t.is_empty()) {
        (false, _) => 0.0,
        (true, true) => 16.0,
        (true, false) => 22.0,
    };
    let x = r.center().x - (g.size().x + li) / 2.0;
    if let Some(i) = i {
        icone(
            p,
            Rect::from_min_size(pos2(x, r.center().y - 8.0), Vec2::splat(16.0)),
            i,
            c,
        );
    }
    p.galley(pos2(x + li, r.center().y - g.size().y / 2.0), g, c);
    anneau_focus(ui, &rep, r, 10.0);
    rep
}

/// Anneau d'accent autour d'un élément qui a le focus clavier.
fn anneau_focus(ui: &Ui, rep: &Response, r: Rect, rayon: f32) {
    if rep.has_focus() {
        ui.painter().rect_stroke(
            r.expand(2.0),
            rayon + 2.0,
            Stroke::new(1.5, ACCENT),
            StrokeKind::Outside,
        );
    }
}

/// Interrupteur animé (40 × 22).
pub fn interrupteur(ui: &mut Ui, on: &mut bool) -> Response {
    let (r, mut rep) = ui.allocate_exact_size(vec2(40.0, 22.0), Sense::click());
    if rep.clicked() {
        *on = !*on;
        rep.mark_changed();
    }
    let a = ui.ctx().animate_bool_with_time(rep.id, *on, 0.16);
    let p = ui.painter();
    if a > 0.0 {
        p.add(degrade(
            r,
            11,
            ACCENT_UV.0,
            ACCENT_UV.1,
            Color32::WHITE.gamma_multiply(a),
        ));
    }
    p.rect_filled(r, 11.0, PISTE.gamma_multiply(1.0 - a));
    p.rect_stroke(
        r,
        11.0,
        Stroke::new(1.0, BORD_FORT.gamma_multiply(1.0 - a)),
        StrokeKind::Inside,
    );
    let x = egui::lerp(r.left() + 11.0..=r.right() - 11.0, a);
    p.circle_filled(
        pos2(x, r.center().y + 1.0),
        8.0,
        Color32::from_black_alpha(50),
    );
    p.circle_filled(pos2(x, r.center().y), 8.0, Color32::WHITE);
    rep
}

/// Contrôle segmenté : une option par segment, indicateur qui glisse.
pub fn segmente<T: PartialEq + Copy>(ui: &mut Ui, v: &mut T, options: &[(T, &str)]) -> Response {
    let l = 96.0;
    let (r, mut rep) =
        ui.allocate_exact_size(vec2(l * options.len() as f32 + 6.0, 34.0), Sense::hover());
    let p = ui.painter().clone();
    p.rect_filled(r, 10.0, CREUX);
    p.rect_stroke(r, 10.0, Stroke::new(1.0, BORD), StrokeKind::Inside);
    let i = options.iter().position(|o| o.0 == *v).unwrap_or(0);
    let x = ui
        .ctx()
        .animate_value_with_time(rep.id.with("glisse"), i as f32, 0.18);
    let sel = Rect::from_min_size(pos2(r.left() + 3.0 + x * l, r.top() + 3.0), vec2(l, 28.0));
    p.rect_filled(sel, 8.0, SURFACE_HAUTE);
    p.rect_stroke(sel, 8.0, Stroke::new(1.0, BORD_FORT), StrokeKind::Inside);
    for (k, (val, nom)) in options.iter().enumerate() {
        let cr = Rect::from_min_size(
            pos2(r.left() + 3.0 + k as f32 * l, r.top() + 3.0),
            vec2(l, 28.0),
        );
        let c = ui.interact(cr, rep.id.with(k), Sense::click());
        if c.clicked() && *v != *val {
            *v = *val;
            rep.mark_changed();
        }
        let col = if k == i {
            TEXTE
        } else if c.hovered() {
            TEXTE2
        } else {
            TEXTE3
        };
        p.text(cr.center(), Align2::CENTER_CENTER, *nom, moyen(13.0), col);
    }
    rep
}

/// Jauge arrondie : creux + remplissage (dégradé de palette `uv`, ou couleur pleine).
pub fn jauge(p: &egui::Painter, r: Rect, frac: f32, uv: Option<(f32, f32)>, c: Color32) {
    let rayon = r.height() / 2.0;
    p.rect_filled(r, rayon, Color32::from_white_alpha(9));
    let w = r.width() * frac.clamp(0.0, 1.0);
    if w < 0.5 {
        return;
    }
    // En dessous de la hauteur, la jauge garde sa forme ronde en rétrécissant.
    let f = Rect::from_min_size(r.min, vec2(w.max(r.height()), r.height()));
    let f = if w < r.height() {
        Rect::from_center_size(f.center(), f.size() * (w / r.height()).max(0.3))
    } else {
        f
    };
    match uv {
        Some((a, b)) => {
            p.add(degrade(f, rayon, a, b, Color32::WHITE));
        }
        None => {
            p.rect_filled(f, rayon, c);
        }
    }
}

/// Carte du fichier entier : un morceau par segment (fait = couleur de sa connexion, reste =
/// la même teinte en transparence), repère ▾ au début de chaque morceau volé.
pub fn carte_fichier(ui: &mut Ui, l: &Live, hauteur: f32) {
    let (zone, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), hauteur + 8.0), Sense::hover());
    let r = Rect::from_min_max(pos2(zone.left(), zone.top() + 8.0), zone.max);
    let p = ui.painter();
    p.rect_filled(r, 4.0, Color32::from_white_alpha(6));
    let taille = l.size.unwrap_or(0).max(1) as f32;
    let x = |o: u64| r.left() + r.width() * (o as f32 / taille).min(1.0);
    let n = l.workers.len();
    for s in &l.segments {
        let c = s.worker.map_or(TEXTE3, |w| couleur_connexion(w, n));
        let fin = s.end.min(l.size.unwrap_or(s.end));
        let (x0, x1) = (x(s.start), x(fin));
        if x1 - x0 < 0.5 {
            continue;
        }
        let ecart = if x1 - x0 > 4.0 { 1.0 } else { 0.0 };
        let tout = Rect::from_x_y_ranges(x0 + ecart..=x1 - ecart, r.y_range());
        p.rect_filled(tout, 3.0, c.gamma_multiply(0.16));
        let xp = x(s.pos).min(tout.right());
        if xp > tout.left() {
            let fini = s.remaining() == 0;
            let fait = Rect::from_x_y_ranges(tout.left()..=xp, r.y_range());
            p.rect_filled(fait, 3.0, if fini { c.gamma_multiply(0.75) } else { c });
        }
        if s.stolen {
            let t = pos2(x0, r.top() - 2.0);
            p.add(Shape::convex_polygon(
                vec![t + vec2(-3.5, -5.0), t + vec2(3.5, -5.0), t],
                TEXTE,
                Stroke::NONE,
            ));
        }
    }
}

/// Une ligne par connexion : numéro, jauge du morceau courant, mention « volé », débit.
pub fn connexions(ui: &mut Ui, l: &Live, vitesses: &[f64], vole: &str, aide_vol: &str) {
    let large = ui.available_width();
    let n = l.workers.len();
    let bv = ui
        .painter()
        .layout_no_wrap(vole.to_string(), moyen(10.5), TEXTE)
        .size()
        .x
        + 14.0;
    for (w, st) in l.workers.iter().enumerate() {
        let (r, rep) = ui.allocate_exact_size(vec2(large, 16.0), Sense::hover());
        // Marge droite : la barre de défilement flottante ne couvre pas les débits.
        let r = r.with_max_x(r.right() - 8.0);
        let c = couleur_connexion(w, n);
        let seg = st.segment.and_then(|i| l.segments.get(i));
        let (frac, actif) = seg.map_or((0.0, false), |s| {
            let n = (s.end - s.start).max(1) as f32;
            ((s.pos - s.start) as f32 / n, s.remaining() > 0)
        });
        let p = ui.painter();
        p.text(
            pos2(r.left() + 18.0, r.center().y),
            Align2::RIGHT_CENTER,
            format!("{}", w + 1),
            regulier(11.5),
            TEXTE3,
        );
        let vole_ici = seg.is_some_and(|s| s.stolen);
        let droite = r.right() - 78.0;
        let barre = Rect::from_x_y_ranges(
            r.left() + 28.0..=droite - if vole_ici { bv + 8.0 } else { 0.0 },
            r.center().y - 3.0..=r.center().y + 3.0,
        );
        let t = rang(w, n);
        let frac = ui
            .ctx()
            .animate_value_with_time(rep.id.with("frac"), frac, 0.45);
        if actif {
            jauge(
                p,
                barre,
                frac,
                Some(((t - 0.07).max(0.0), (t + 0.05).min(1.0))),
                c,
            );
        } else {
            jauge(p, barre, frac, None, c.gamma_multiply(0.35));
        }
        if vole_ici {
            let b = Rect::from_min_size(pos2(droite - bv, r.center().y - 8.0), vec2(bv, 16.0));
            p.rect_filled(b, 8.0, c.gamma_multiply(0.15));
            p.text(b.center(), Align2::CENTER_CENTER, vole, moyen(10.5), c);
            ui.interact(b, rep.id.with("vol"), Sense::hover())
                .on_hover_text(aide_vol);
        }
        let v = vitesses.get(w).copied().unwrap_or(0.0);
        ui.painter().text(
            pos2(r.right(), r.center().y),
            Align2::RIGHT_CENTER,
            if actif {
                format!("{}/s", super::octets(v as u64))
            } else {
                "—".into()
            },
            regulier(12.0),
            if actif { TEXTE2 } else { TEXTE3 },
        );
    }
}

/// Courbe du débit total : ligne lissée au dégradé de palette, remplissage qui s'efface.
pub fn courbe(ui: &mut Ui, h: &[f32], max: f32, hauteur: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), hauteur), Sense::hover());
    let p = ui.painter();
    for k in 0..4 {
        let y = r.top() + r.height() * k as f32 / 3.0;
        p.hline(
            r.x_range(),
            y.round() + 0.5,
            Stroke::new(1.0, Color32::from_white_alpha(7)),
        );
    }
    if h.len() < 2 {
        return;
    }
    let max = max.max(1.0);
    let brut: Vec<Pos2> = h
        .iter()
        .enumerate()
        .map(|(i, v)| {
            pos2(
                r.left() + r.width() * i as f32 / (h.len() - 1) as f32,
                r.bottom() - (r.height() - 8.0) * (v / max).clamp(0.0, 1.0),
            )
        })
        .collect();
    // Catmull-Rom : 4 points intermédiaires par intervalle.
    let mut pts = Vec::with_capacity(brut.len() * 4);
    for i in 0..brut.len() - 1 {
        let p0 = brut[i.saturating_sub(1)];
        let (p1, p2) = (brut[i], brut[i + 1]);
        let p3 = brut[(i + 2).min(brut.len() - 1)];
        for k in 0..4 {
            let t = k as f32 / 4.0;
            let (t2, t3) = (t * t, t * t * t);
            let f = |a: f32, b: f32, c: f32, d: f32| {
                0.5 * (2.0 * b
                    + (c - a) * t
                    + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
                    + (3.0 * b - a - 3.0 * c + d) * t3)
            };
            let y = f(p0.y, p1.y, p2.y, p3.y).clamp(r.top(), r.bottom());
            pts.push(pos2(f(p0.x, p1.x, p2.x, p3.x), y));
        }
    }
    pts.push(*brut.last().unwrap_or(&r.right_bottom()));
    let mut m = Mesh::default();
    for q in &pts {
        let t = (q.x - r.left()) / r.width();
        let c = teinte(0.3 + 0.55 * t);
        let haut = 1.0 - (q.y - r.top()) / r.height();
        m.colored_vertex(*q, c.gamma_multiply(0.10 + 0.22 * haut));
        m.colored_vertex(pos2(q.x, r.bottom()), Color32::TRANSPARENT);
    }
    for i in 0..pts.len() as u32 - 1 {
        let a = i * 2;
        m.add_triangle(a, a + 1, a + 2);
        m.add_triangle(a + 1, a + 2, a + 3);
    }
    p.add(Shape::mesh(m));
    let gauche = r.left();
    let large = r.width();
    p.add(Shape::line(
        pts.clone(),
        egui::epaint::PathStroke::new_uv(2.0, move |_, q| {
            teinte(0.3 + 0.55 * ((q.x - gauche) / large))
        }),
    ));
    if let Some(d) = pts.last() {
        let c = teinte(0.85);
        p.circle_filled(*d, 7.0, c.gamma_multiply(0.18));
        p.circle_filled(*d, 3.5, c);
        p.circle_stroke(*d, 3.5, Stroke::new(1.5, FOND));
    }
}

/// Curseur : rail fin, remplissage au dégradé d'accent, pastille blanche. Flèches du clavier
/// au focus (pas de `pas`). `log` : échelle logarithmique (min > 0).
pub fn curseur(
    ui: &mut Ui,
    v: &mut f64,
    (min, max): (f64, f64),
    log: bool,
    pas: f64,
    largeur: f32,
) -> Response {
    let (r, mut rep) = ui.allocate_exact_size(vec2(largeur, 24.0), Sense::click_and_drag());
    let norm = |x: f64| {
        if log {
            (x.max(min).ln() - min.ln()) / (max.ln() - min.ln())
        } else {
            (x - min) / (max - min)
        }
    };
    let denorm = |t: f64| {
        if log {
            (min.ln() + t * (max.ln() - min.ln())).exp()
        } else {
            min + t * (max - min)
        }
    };
    let rail = r.shrink2(vec2(9.0, 0.0));
    let mut nv = *v;
    if let Some(p) = rep.interact_pointer_pos() {
        if rep.dragged() || rep.clicked() {
            nv = denorm(((p.x - rail.left()) / rail.width()).clamp(0.0, 1.0) as f64);
        }
    }
    if rep.has_focus() {
        let (g, d) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
            )
        });
        if g {
            nv -= pas;
        }
        if d {
            nv += pas;
        }
    }
    let nv = nv.clamp(min, max);
    if nv != *v {
        *v = nv;
        rep.mark_changed();
    }
    let t = norm(*v).clamp(0.0, 1.0) as f32;
    let p = ui.painter();
    let piste = Rect::from_x_y_ranges(rail.x_range(), r.center().y - 2.0..=r.center().y + 2.0);
    p.rect_filled(piste, 2.0, PISTE);
    let x = rail.left() + rail.width() * t;
    if x > rail.left() + 1.0 {
        p.add(degrade(
            Rect::from_x_y_ranges(rail.left()..=x, piste.y_range()),
            2,
            ACCENT_UV.0,
            ACCENT_UV.1,
            Color32::WHITE,
        ));
    }
    let h = ui
        .ctx()
        .animate_bool_with_time(rep.id, rep.hovered() || rep.dragged(), 0.12);
    let c = pos2(x, r.center().y);
    p.circle_filled(c + vec2(0.0, 1.5), 8.5, Color32::from_black_alpha(70));
    if h > 0.0 {
        p.circle_filled(c, 8.0 + 5.0 * h, ACCENT.gamma_multiply(0.18 * h));
    }
    p.circle_filled(c, 8.0, Color32::WHITE);
    if rep.has_focus() {
        p.circle_stroke(c, 11.0, Stroke::new(1.5, ACCENT));
    }
    rep
}

/// Petite boîte de valeur (texte) à côté d'un curseur.
pub fn boite_valeur(ui: &mut Ui, t: &str, largeur: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(largeur, 30.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, 8.0, CREUX);
    p.rect_stroke(r, 8.0, Stroke::new(1.0, BORD), StrokeKind::Inside);
    p.text(r.center(), Align2::CENTER_CENTER, t, moyen(13.0), TEXTE);
}
