//! Style (polices pixel OFL, jetons de couleur) et dessins : carte du fichier, une barre par
//! connexion, courbe de débit.

use dlcore::engine::Live;
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontFamily, FontId, Margin, Pos2, Rect, Sense, Stroke,
    StrokeKind, Ui, Vec2,
};
use std::sync::Arc;

pub const FOND: Color32 = Color32::from_rgb(0x0a, 0x0e, 0x16);
pub const PANNEAU: Color32 = Color32::from_rgb(0x12, 0x18, 0x24);
pub const CREUX: Color32 = Color32::from_rgb(0x07, 0x0a, 0x10);
pub const LIGNE: Color32 = Color32::from_rgb(0x26, 0x32, 0x46);
pub const TEXTE: Color32 = Color32::from_rgb(0xe6, 0xed, 0xf3);
pub const DISCRET: Color32 = Color32::from_rgb(0x7d, 0x8b, 0x9e);
pub const CYAN: Color32 = Color32::from_rgb(0x3e, 0xe0, 0xd0);
pub const ROSE: Color32 = Color32::from_rgb(0xff, 0x4f, 0xa3);
pub const AMBRE: Color32 = Color32::from_rgb(0xff, 0xc8, 0x57);
pub const VERT: Color32 = Color32::from_rgb(0x5d, 0xf0, 0x8a);
pub const ALERTE: Color32 = Color32::from_rgb(0xff, 0x5d, 0x73);

pub fn titre(taille: f32) -> FontId {
    FontId::new(taille, FontFamily::Name("titre".into()))
}

pub fn installer_style(ctx: &egui::Context) {
    let mut polices = egui::FontDefinitions::empty();
    for (nom, octets) in [
        (
            "pixelify",
            &include_bytes!("../../assets/fonts/PixelifySans-Regular.ttf")[..],
        ),
        (
            "silkscreen",
            &include_bytes!("../../assets/fonts/Silkscreen-Regular.ttf")[..],
        ),
    ] {
        polices
            .font_data
            .insert(nom.into(), Arc::new(egui::FontData::from_static(octets)));
    }
    for (famille, ordre) in [
        (FontFamily::Proportional, ["pixelify", "silkscreen"]),
        (FontFamily::Monospace, ["pixelify", "silkscreen"]),
        (FontFamily::Name("titre".into()), ["silkscreen", "pixelify"]),
    ] {
        polices
            .families
            .insert(famille, ordre.iter().map(|s| s.to_string()).collect());
    }
    ctx.set_fonts(polices);
    let mut style = (*ctx.global_style()).clone();
    style.text_styles = [
        (egui::TextStyle::Small, FontId::proportional(13.0)),
        (egui::TextStyle::Body, FontId::proportional(16.0)),
        (egui::TextStyle::Button, FontId::proportional(16.0)),
        (egui::TextStyle::Heading, titre(20.0)),
        (egui::TextStyle::Monospace, FontId::proportional(16.0)),
    ]
    .into();
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(10.0, 5.0);
    style.spacing.slider_width = 260.0;
    let v = &mut style.visuals;
    *v = egui::Visuals::dark();
    v.override_text_color = Some(TEXTE);
    v.panel_fill = FOND;
    v.window_fill = PANNEAU;
    v.extreme_bg_color = CREUX;
    v.faint_bg_color = PANNEAU;
    v.selection.bg_fill = Color32::from_rgb(0x1d, 0x6f, 0x6a);
    v.selection.stroke = Stroke::new(2.0, CYAN);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Rect { aspect_ratio: 0.6 };
    for (w, fond) in [
        (&mut v.widgets.inactive, Color32::from_rgb(0x1a, 0x24, 0x34)),
        (&mut v.widgets.hovered, Color32::from_rgb(0x22, 0x30, 0x46)),
        (&mut v.widgets.active, Color32::from_rgb(0x2a, 0x3c, 0x58)),
        (&mut v.widgets.open, Color32::from_rgb(0x22, 0x30, 0x46)),
        (&mut v.widgets.noninteractive, PANNEAU),
    ] {
        w.bg_fill = fond;
        w.weak_bg_fill = fond;
        w.corner_radius = CornerRadius::ZERO;
        w.expansion = 0.0;
        w.fg_stroke = Stroke::new(1.5, TEXTE);
    }
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, LIGNE);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, CYAN);
    v.widgets.active.bg_stroke = Stroke::new(1.0, TEXTE);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LIGNE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, DISCRET);
    v.window_corner_radius = CornerRadius::ZERO;
    v.menu_corner_radius = CornerRadius::ZERO;
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_global_style(style);
}

/// Cadre de carte : fond panneau, liseré, ombre pixel nette.
pub fn carte(liseré: Color32) -> egui::Frame {
    egui::Frame::new()
        .fill(PANNEAU)
        .stroke(Stroke::new(1.0, liseré))
        .inner_margin(Margin::same(12))
        .shadow(egui::Shadow {
            offset: [3, 3],
            blur: 0,
            spread: 0,
            color: CREUX,
        })
}

/// Couleur d'une connexion : teintes réparties au nombre d'or, vives sur fond sombre.
pub fn couleur(w: usize) -> Color32 {
    let h = (w as f32 * 0.618_034 + 0.47).fract();
    egui::ecolor::Hsva::new(h, 0.72, 0.98, 1.0).into()
}

fn atténue(c: Color32, f: f32) -> Color32 {
    Color32::from_rgb(
        (c.r() as f32 * f) as u8,
        (c.g() as f32 * f) as u8,
        (c.b() as f32 * f) as u8,
    )
}

/// Barre pixelisée : creux, remplissage par cases de 4 px.
fn barre(ui: &Ui, r: Rect, frac: f32, c: Color32) {
    let p = ui.painter();
    p.rect_filled(r, 0.0, CREUX);
    let w = (r.width() * frac.clamp(0.0, 1.0) / 4.0).round() * 4.0;
    if w > 0.0 {
        p.rect_filled(Rect::from_min_size(r.min, Vec2::new(w, r.height())), 0.0, c);
        // Reflet d'une ligne en haut, façon jauge rétro.
        p.rect_filled(
            Rect::from_min_size(r.min, Vec2::new(w, 2.0)),
            0.0,
            Color32::from_white_alpha(70),
        );
    }
    p.rect_stroke(r, 0.0, Stroke::new(1.0, LIGNE), StrokeKind::Inside);
}

/// Carte du fichier entier : chaque morceau fait prend la couleur de sa connexion.
pub fn carte_fichier(ui: &mut Ui, l: &Live, hauteur: f32) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), hauteur), Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, 0.0, CREUX);
    let taille = l.size.unwrap_or(0).max(1) as f32;
    let x = |o: u64| r.left() + r.width() * (o as f32 / taille).min(1.0);
    for s in &l.segments {
        let c = s.worker.map_or(DISCRET, couleur);
        let fin = s.end.min(l.size.unwrap_or(s.end));
        let fait = Rect::from_x_y_ranges(x(s.start)..=x(s.pos), r.y_range());
        p.rect_filled(
            fait,
            0.0,
            if s.remaining() == 0 {
                atténue(c, 0.75)
            } else {
                c
            },
        );
        let reste = Rect::from_x_y_ranges(x(s.pos)..=x(fin), r.y_range());
        p.rect_filled(reste, 0.0, atténue(c, 0.18));
        p.vline(x(s.start), r.y_range(), Stroke::new(1.0, FOND));
    }
    p.rect_stroke(r, 0.0, Stroke::new(1.0, LIGNE), StrokeKind::Inside);
}

/// Une ligne par connexion : numéro, barre de son morceau courant, débit, badge « ½ » si volé.
pub fn connexions(ui: &mut Ui, l: &Live, vitesses: &[f64], volé: &str) {
    let large = ui.available_width();
    ui.spacing_mut().item_spacing.y = 5.0;
    for (w, st) in l.workers.iter().enumerate() {
        let (r, _) = ui.allocate_exact_size(Vec2::new(large, 16.0), Sense::hover());
        let c = couleur(w);
        let p = ui.painter();
        p.text(
            Pos2::new(r.left(), r.center().y),
            Align2::LEFT_CENTER,
            format!("#{:02}", w + 1),
            titre(11.0),
            c,
        );
        let zone = Rect::from_x_y_ranges(r.left() + 36.0..=r.right() - 84.0, r.y_range());
        let seg = st.segment.and_then(|i| l.segments.get(i));
        let (frac, actif) = seg.map_or((0.0, false), |s| {
            let n = (s.end - s.start).max(1) as f32;
            ((s.pos - s.start) as f32 / n, s.remaining() > 0)
        });
        barre(ui, zone, frac, if actif { c } else { atténue(c, 0.45) });
        if seg.is_some_and(|s| s.stolen) {
            let b = Rect::from_min_size(
                Pos2::new(zone.right() - 54.0, zone.top() + 2.0),
                Vec2::new(52.0, zone.height() - 4.0),
            );
            let p = ui.painter();
            p.rect_filled(b, 0.0, AMBRE);
            p.text(b.center(), Align2::CENTER_CENTER, volé, titre(8.0), FOND);
        }
        let v = vitesses.get(w).copied().unwrap_or(0.0);
        ui.painter().text(
            Pos2::new(r.right(), r.center().y),
            Align2::RIGHT_CENTER,
            if actif {
                format!("{}/s", super::octets(v as u64))
            } else {
                "—".into()
            },
            FontId::proportional(14.0),
            if actif { TEXTE } else { DISCRET },
        );
    }
}

/// Courbe du débit total (historique récent).
pub fn courbe(ui: &mut Ui, h: &[f32], hauteur: f32) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), hauteur), Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, 0.0, CREUX);
    for k in 1..4 {
        let y = r.top() + r.height() * k as f32 / 4.0;
        p.hline(
            r.x_range(),
            y,
            Stroke::new(1.0, Color32::from_rgb(0x14, 0x1c, 0x2a)),
        );
    }
    let max = h.iter().copied().fold(1.0, f32::max);
    if h.len() >= 2 {
        let pts: Vec<Pos2> = h
            .iter()
            .enumerate()
            .map(|(i, v)| {
                Pos2::new(
                    r.left() + r.width() * i as f32 / (h.len() - 1) as f32,
                    r.bottom() - 2.0 - (r.height() - 6.0) * v / max,
                )
            })
            .collect();
        for (a, b) in pts.iter().zip(pts.iter().skip(1)) {
            p.add(egui::Shape::convex_polygon(
                vec![
                    *a,
                    *b,
                    Pos2::new(b.x, r.bottom()),
                    Pos2::new(a.x, r.bottom()),
                ],
                Color32::from_rgba_unmultiplied(0x3e, 0xe0, 0xd0, 28),
                Stroke::NONE,
            ));
        }
        p.add(egui::Shape::line(pts, Stroke::new(2.0, CYAN)));
    }
    p.text(
        r.left_top() + Vec2::new(6.0, 4.0),
        Align2::LEFT_TOP,
        format!("{}/s", super::octets(max as u64)),
        FontId::proportional(12.0),
        DISCRET,
    );
    p.rect_stroke(r, 0.0, Stroke::new(1.0, LIGNE), StrokeKind::Inside);
}

/// Barre de comparatif : longueur relative au meilleur débit.
pub fn barre_comparatif(ui: &mut Ui, frac: f32, c: Color32) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::hover());
    barre(ui, r, frac, c);
}

/// Petite barre de progression d'une carte de la file.
pub fn mini(ui: &mut Ui, frac: f32, c: Color32) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 8.0), Sense::hover());
    barre(ui, r, frac, c);
}
