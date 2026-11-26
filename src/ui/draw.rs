//! Style « Frutiger Aero » (Open Sans OFL, fond ciel et eau, verre dépoli, boutons gel) et
//! dessins : icônes vectorielles, boutons, jauges brillantes, carte du fichier, une barre par
//! connexion, courbe de débit.

use dlcore::engine::Live;
use eframe::egui::{
    self, epaint::RectShape, pos2, vec2, Align2, Color32, CornerRadius, FontFamily, FontId, Margin,
    Mesh, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, TextureId, Ui, Vec2,
};
use std::sync::{Arc, OnceLock};

pub const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// Bleu nuit translucide (ombres, liserés) : `a` = opacité 0..255.
pub const fn nuit(a: u8) -> Color32 {
    // Prémultiplié à la main : (14, 42, 78) × a / 255.
    Color32::from_rgba_premultiplied(
        (14 * a as u16 / 255) as u8,
        (42 * a as u16 / 255) as u8,
        (78 * a as u16 / 255) as u8,
        a,
    )
}

pub fn blanc(a: u8) -> Color32 {
    Color32::from_white_alpha(a)
}

/// Texte sur verre clair : bleu nuit, puis deux gris bleutés.
pub const TEXTE: Color32 = rgb(0x0f, 0x2a, 0x45);
pub const TEXTE2: Color32 = rgb(0x3a, 0x55, 0x72);
pub const TEXTE3: Color32 = rgb(0x5f, 0x76, 0x8f);
/// Bleu des titres (panneaux Vista / Windows 7).
pub const TITRE: Color32 = rgb(0x15, 0x4f, 0x95);
pub const ACCENT: Color32 = rgb(0x1e, 0x78, 0xd6);
pub const SUCCES: Color32 = rgb(0x2c, 0x9a, 0x1e);
pub const ATTENTION: Color32 = rgb(0xd0, 0x7a, 0x0c);
pub const DANGER: Color32 = rgb(0xd2, 0x33, 0x33);
pub const BORD: Color32 = nuit(46);
pub const BORD_FORT: Color32 = nuit(90);

/// Palette des connexions : vert frais → turquoise → aqua → bleu ciel → bleu profond.
const ARRETS: [(f32, Color32); 5] = [
    (0.0, rgb(0x4c, 0xc7, 0x2b)),
    (0.28, rgb(0x1e, 0xbd, 0x8a)),
    (0.52, rgb(0x12, 0xac, 0xd6)),
    (0.78, rgb(0x2a, 0x7e, 0xe0)),
    (1.0, rgb(0x3f, 0x5f, 0xd6)),
];

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

/// Textures du style : profils d'opacité verticaux (blanc) et fond net / flouté.
struct Textures {
    /// Opacité 1 → 0 de haut en bas : dégradés verticaux.
    rampe: TextureId,
    /// Reflet du verre : fort en haut, coupé net à mi-hauteur.
    reflet: TextureId,
    fond: TextureId,
    flou: TextureId,
    /// Largeur / hauteur de l'image de fond.
    ratio: f32,
}

static TEX: OnceLock<Textures> = OnceLock::new();

pub fn regulier(t: f32) -> FontId {
    FontId::proportional(t)
}
pub fn moyen(t: f32) -> FontId {
    FontId::new(t, FontFamily::Name("moyen".into()))
}
pub fn demi(t: f32) -> FontId {
    FontId::new(t, FontFamily::Name("demi".into()))
}
/// Grands titres et grands chiffres : Open Sans Light (l'esprit Segoe UI Light de Vista).
pub fn affiche(t: f32) -> FontId {
    FontId::new(t, FontFamily::Name("affiche".into()))
}

/// Polices de l'appli ; `cjk` : police du système pour le chinois, le japonais et le coréen
/// (trop lourde à embarquer), ajoutée en dernier recours.
fn polices(cjk: Option<Vec<u8>>) -> egui::FontDefinitions {
    let mut polices = egui::FontDefinitions::empty();
    // Open Sans (OFL), sous-ensemble latin + grec + cyrillique sans hinting ; chiffres
    // tabulaires d'origine. Flèches et quelques symboles : DejaVu Sans (licence Bitstream Vera).
    for (nom, octets) in [
        (
            "os",
            &include_bytes!("../../assets/fonts/OpenSans-Regular.ttf")[..],
        ),
        (
            "os-moyen",
            &include_bytes!("../../assets/fonts/OpenSans-Medium.ttf")[..],
        ),
        (
            "os-demi",
            &include_bytes!("../../assets/fonts/OpenSans-SemiBold.ttf")[..],
        ),
        (
            "os-fin",
            &include_bytes!("../../assets/fonts/OpenSans-Light.ttf")[..],
        ),
        (
            "symboles",
            &include_bytes!("../../assets/fonts/DejaVuSans-Symboles.ttf")[..],
        ),
    ] {
        polices
            .font_data
            .insert(nom.into(), Arc::new(egui::FontData::from_static(octets)));
    }
    for (famille, nom) in [
        (FontFamily::Proportional, "os"),
        (FontFamily::Monospace, "os"),
        (FontFamily::Name("moyen".into()), "os-moyen"),
        (FontFamily::Name("demi".into()), "os-demi"),
        (FontFamily::Name("affiche".into()), "os-fin"),
    ] {
        let mut l = vec![nom.to_string(), "os".to_string(), "symboles".to_string()];
        if cjk.is_some() {
            l.push("cjk".to_string());
        }
        polices.families.insert(famille, l);
    }
    if let Some(o) = cjk {
        polices
            .font_data
            .insert("cjk".into(), Arc::new(egui::FontData::from_owned(o)));
    }
    polices
}

/// Texte en écriture chinoise, japonaise ou coréenne (absente des polices embarquées).
pub fn besoin_cjk(s: &str) -> bool {
    s.chars().any(|c| {
        matches!(c as u32, 0x2E80..=0x9FFF | 0xA960..=0xA97F | 0xAC00..=0xD7FF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x20000..=0x2FA1F)
    })
}

/// Charge une police CJK du système (une seule fois, au premier nom qui en a besoin : ~20 Mo
/// en mémoire). Vrai si elle est en place.
pub fn charger_cjk(ctx: &egui::Context) -> bool {
    #[cfg(windows)]
    let essais: Vec<std::path::PathBuf> = {
        let f = std::path::PathBuf::from(
            std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into()),
        )
        .join("Fonts");
        [
            "msyh.ttc",
            "YuGothR.ttc",
            "malgun.ttf",
            "msgothic.ttc",
            "simsun.ttc",
        ]
        .iter()
        .map(|n| f.join(n))
        .collect()
    };
    #[cfg(not(windows))]
    let essais: Vec<std::path::PathBuf> = [
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
    ]
    .iter()
    .map(std::path::PathBuf::from)
    .collect();
    let Some(o) = essais.iter().find_map(|p| std::fs::read(p).ok()) else {
        return false;
    };
    ctx.set_fonts(polices(Some(o)));
    true
}

/// Polices, visuels egui, textures du style. Renvoie les textures (à garder en vie).
pub fn installer_style(ctx: &egui::Context) -> Vec<egui::TextureHandle> {
    ctx.set_fonts(polices(None));
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
    *v = egui::Visuals::light();
    v.override_text_color = Some(TEXTE);
    v.panel_fill = Color32::TRANSPARENT;
    // Infobulles et menus : verre blanc presque opaque, liseré bleu nuit.
    v.window_fill = rgb(0xf7, 0xfa, 0xfe);
    v.window_stroke = Stroke::new(1.0, BORD_FORT);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(8);
    v.popup_shadow = egui::Shadow {
        offset: [0, 4],
        blur: 14,
        spread: 0,
        color: nuit(60),
    };
    v.extreme_bg_color = Color32::WHITE;
    v.faint_bg_color = blanc(120);
    v.selection.bg_fill = rgb(0xb4, 0xd8, 0xf8);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.text_cursor.stroke = Stroke::new(1.5, ACCENT);
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    // Widgets egui restants (DragValue) : boutons Windows 7.
    for (w, fond, bord, texte) in [
        (
            &mut v.widgets.inactive,
            rgb(0xf6, 0xf9, 0xfc),
            rgb(0xa3, 0xb3, 0xc6),
            TEXTE,
        ),
        (
            &mut v.widgets.hovered,
            rgb(0xe3, 0xf2, 0xfd),
            rgb(0x3c, 0x7f, 0xb1),
            TEXTE,
        ),
        (
            &mut v.widgets.active,
            rgb(0xc7, 0xe5, 0xf8),
            rgb(0x2c, 0x62, 0x8b),
            TEXTE,
        ),
        (
            &mut v.widgets.open,
            rgb(0xe3, 0xf2, 0xfd),
            rgb(0x3c, 0x7f, 0xb1),
            TEXTE,
        ),
        (&mut v.widgets.noninteractive, blanc(150), BORD, TEXTE2),
    ] {
        w.bg_fill = fond;
        w.weak_bg_fill = fond;
        w.bg_stroke = Stroke::new(1.0, bord);
        w.corner_radius = CornerRadius::same(6);
        w.expansion = 0.0;
        w.fg_stroke = Stroke::new(1.5, texte);
    }
    ctx.set_theme(egui::Theme::Light);
    ctx.set_global_style(style);

    let profil = |nom: &str, f: &dyn Fn(f32) -> f32| {
        let img = egui::ColorImage::new(
            [1, 64],
            (0..64)
                .map(|i| {
                    let a = (f((i as f32 + 0.5) / 64.0).clamp(0.0, 1.0) * 255.0).round() as u8;
                    Color32::from_white_alpha(a)
                })
                .collect(),
        );
        ctx.load_texture(nom, img, egui::TextureOptions::LINEAR)
    };
    let rampe = profil("rampe", &|y| 1.0 - y);
    let reflet = profil("reflet", &|y| {
        if y < 0.5 {
            0.92 - 0.62 * (y / 0.5)
        } else {
            0.0
        }
    });
    // Fond : JPEG embarqué ; version floutée (réduite 1/8) pour le verre dépoli.
    let img = image::load_from_memory_with_format(
        include_bytes!("../../assets/fond.jpg"),
        image::ImageFormat::Jpeg,
    )
    .map(|i| i.to_rgba8())
    .unwrap_or_else(|_| image::RgbaImage::from_pixel(16, 10, image::Rgba([90, 170, 230, 255])));
    let (w, h) = img.dimensions();
    let petit = image::imageops::resize(
        &img,
        (w / 8).max(1),
        (h / 8).max(1),
        image::imageops::FilterType::Triangle,
    );
    let flou = image::imageops::blur(&petit, 3.0);
    let charger = |nom: &str, i: &image::RgbaImage, mip: bool| {
        let c = egui::ColorImage::from_rgba_unmultiplied(
            [i.width() as usize, i.height() as usize],
            i.as_raw(),
        );
        let mut o = egui::TextureOptions::LINEAR;
        if mip {
            o.mipmap_mode = Some(egui::TextureFilter::Linear);
        }
        ctx.load_texture(nom, c, o)
    };
    let fond = charger("fond", &img, true);
    let flou = charger("fond-flou", &flou, false);
    let _ = TEX.set(Textures {
        rampe: rampe.id(),
        reflet: reflet.id(),
        fond: fond.id(),
        flou: flou.id(),
        ratio: w as f32 / h as f32,
    });
    vec![rampe, reflet, fond, flou]
}

/// Portion de l'image de fond (coordonnées de texture) sous `r`, l'image couvrant `ecran`.
fn uv_fond(ecran: Rect, ratio: f32, r: Rect) -> Rect {
    let re = ecran.width() / ecran.height().max(1.0);
    let (uw, vh) = if re > ratio {
        (1.0, ratio / re)
    } else {
        (re / ratio, 1.0)
    };
    let (u0, v0) = ((1.0 - uw) / 2.0, (1.0 - vh) / 2.0);
    let u = |x: f32| u0 + (x - ecran.left()) / ecran.width() * uw;
    let v = |y: f32| v0 + (y - ecran.top()) / ecran.height() * vh;
    Rect::from_min_max(
        pos2(u(r.left()), v(r.top())),
        pos2(u(r.right()), v(r.bottom())),
    )
}

/// Fond d'écran (ciel, eau, herbe) étiré pour couvrir `ecran`.
pub fn fond(p: &egui::Painter, ecran: Rect) {
    let Some(t) = TEX.get() else {
        p.rect_filled(ecran, 0.0, rgb(0x5a, 0xaa, 0xe6));
        return;
    };
    p.add(
        RectShape::filled(ecran, 0, Color32::WHITE)
            .with_texture(t.fond, uv_fond(ecran, t.ratio, ecran)),
    );
}

/// Dégradé vertical arrondi `haut` → `bas` (couleur pleine + rampe d'opacité teintée).
pub fn vdeg(r: Rect, rayon: impl Into<CornerRadius>, haut: Color32, bas: Color32) -> Shape {
    let rayon = rayon.into();
    let Some(t) = TEX.get() else {
        return Shape::rect_filled(r, rayon, mix(haut, bas, 0.5));
    };
    Shape::Vec(vec![
        Shape::rect_filled(r, rayon, bas),
        RectShape::filled(r, rayon, haut)
            .with_texture(t.rampe, Rect::from_min_max(pos2(0.5, 0.0), pos2(0.5, 1.0)))
            .into(),
    ])
}

/// Reflet du verre sur la moitié haute de `r` (opacité `a` 0..1).
pub fn reflet(r: Rect, rayon: impl Into<CornerRadius>, a: f32) -> Shape {
    let Some(t) = TEX.get() else {
        return Shape::Noop;
    };
    RectShape::filled(r, rayon, Color32::WHITE.gamma_multiply(a))
        .with_texture(t.reflet, Rect::from_min_max(pos2(0.5, 0.0), pos2(0.5, 1.0)))
        .into()
}

/// Lueur douce (rectangle aux bords flous).
pub fn lueur(r: Rect, c: Color32, flou: f32) -> Shape {
    RectShape::filled(r, r.height().min(r.width()) / 2.0, c)
        .with_blur_width(flou)
        .into()
}

/// Panneau de verre dépoli : ombre portée, fond flouté vu à travers, voile blanc `voile`
/// (0..1) plus clair en haut, liseré clair dedans et sombre dehors.
pub fn verre(ctx: &egui::Context, r: Rect, rayon: u8, voile: f32) -> Shape {
    let mut v = vec![egui::Shadow {
        offset: [0, 8],
        blur: 26,
        spread: 0,
        color: nuit(55),
    }
    .as_shape(r, rayon)
    .into()];
    if let Some(t) = TEX.get() {
        let ecran = ctx.content_rect();
        v.push(
            RectShape::filled(r, rayon, Color32::WHITE)
                .with_texture(t.flou, uv_fond(ecran, t.ratio, r))
                .into(),
        );
    }
    v.push(Shape::rect_filled(r, rayon, blanc((voile * 255.0) as u8)));
    v.push(vdeg(r, rayon, blanc(70), Color32::TRANSPARENT));
    // Lustre du haut : bande claire qui s'efface.
    let bande = Rect::from_min_size(r.min, vec2(r.width(), (r.height() * 0.5).min(56.0)));
    let cr = CornerRadius {
        nw: rayon,
        ne: rayon,
        sw: 0,
        se: 0,
    };
    v.push(vdeg(bande, cr, blanc(110), Color32::TRANSPARENT));
    v.push(Shape::rect_stroke(
        r.shrink(1.0),
        rayon.saturating_sub(1),
        Stroke::new(1.0, blanc(200)),
        StrokeKind::Inside,
    ));
    v.push(Shape::rect_stroke(
        r,
        rayon,
        Stroke::new(1.0, nuit(70)),
        StrokeKind::Inside,
    ));
    Shape::Vec(v)
}

/// Carte de verre autour du contenu ajouté par `f`.
pub fn carte<R>(ui: &mut Ui, marge: Margin, f: impl FnOnce(&mut Ui) -> R) -> R {
    let fond = ui.painter().add(Shape::Noop);
    let rep = egui::Frame::NONE.inner_margin(marge).show(ui, f);
    let r = rep.response.rect;
    ui.painter().set(fond, verre(ui.ctx(), r, 14, 0.62));
    rep.inner
}

/// Teintes d'un bouton gel : haut et bas du dégradé, liseré, lueur du bas.
#[derive(Clone, Copy)]
pub struct Gel {
    pub haut: Color32,
    pub bas: Color32,
    pub bord: Color32,
    pub lueur: Color32,
}

pub const GEL_BLEU: Gel = Gel {
    haut: rgb(0x5c, 0xb6, 0xf6),
    bas: rgb(0x0b, 0x5c, 0xc2),
    bord: rgb(0x0a, 0x47, 0x93),
    lueur: rgb(0x8c, 0xf0, 0xff),
};
pub const GEL_VERT: Gel = Gel {
    haut: rgb(0x8f, 0xe2, 0x5c),
    bas: rgb(0x2b, 0x8f, 0x16),
    bord: rgb(0x23, 0x6e, 0x12),
    lueur: rgb(0xdc, 0xff, 0x8e),
};
const GEL_ARGENT: Gel = Gel {
    haut: rgb(0xfe, 0xfe, 0xff),
    bas: rgb(0xd3, 0xdb, 0xe5),
    bord: rgb(0x8d, 0x9e, 0xb3),
    lueur: rgb(0xff, 0xff, 0xff),
};
/// Argent survolé : le bleu clair des boutons Windows 7.
const GEL_ARGENT_SURVOL: Gel = Gel {
    haut: rgb(0xf0, 0xf9, 0xff),
    bas: rgb(0xb3, 0xdc, 0xf8),
    bord: rgb(0x3c, 0x7f, 0xb1),
    lueur: rgb(0xff, 0xff, 0xff),
};

/// Gel tiré d'une couleur (jauges, pastilles).
pub fn gel_de(c: Color32) -> Gel {
    Gel {
        haut: mix(c, Color32::WHITE, 0.42),
        bas: mix(c, Color32::BLACK, 0.1),
        bord: mix(c, Color32::BLACK, 0.38),
        lueur: mix(c, Color32::WHITE, 0.62),
    }
}

fn gel_mix(a: Gel, b: Gel, t: f32) -> Gel {
    Gel {
        haut: mix(a.haut, b.haut, t),
        bas: mix(a.bas, b.bas, t),
        bord: mix(a.bord, b.bord, t),
        lueur: mix(a.lueur, b.lueur, t),
    }
}

/// Surface gel : dégradé, lueur intérieure en bas, reflet en haut, double liseré.
/// `ombre` : petite ombre portée (boutons) ; `presse` : enfoncé (reflet atténué).
pub fn gel(p: &egui::Painter, r: Rect, rayon: f32, g: Gel, ombre: bool, presse: bool) {
    if ombre {
        p.add(
            egui::Shadow {
                offset: [0, 2],
                blur: 6,
                spread: 0,
                color: nuit(if presse { 25 } else { 50 }),
            }
            .as_shape(r, rayon),
        );
    }
    let (haut, bas) = if presse {
        (mix(g.haut, g.bas, 0.35), g.bas)
    } else {
        (g.haut, g.bas)
    };
    p.add(vdeg(r, rayon, haut, bas));
    if r.height() >= 10.0 {
        let l = Rect::from_center_size(
            pos2(r.center().x, r.bottom() - r.height() * 0.12),
            vec2(
                (r.width() - r.height() * 0.6).max(r.width() * 0.6),
                r.height() * 0.5,
            ),
        );
        p.with_clip_rect(r.shrink(1.0)).add(lueur(
            l,
            g.lueur.gamma_multiply(0.55),
            r.height() * 0.45,
        ));
    }
    let i = r.shrink2(vec2(1.5, 1.0));
    p.add(reflet(
        i,
        (rayon - 1.0).max(0.0),
        if presse { 0.45 } else { 0.95 },
    ));
    if r.height() >= 8.0 {
        p.rect_stroke(
            r.shrink(1.0),
            (rayon - 1.0).max(0.0),
            Stroke::new(1.0, blanc(if presse { 40 } else { 95 })),
            StrokeKind::Inside,
        );
    }
    p.rect_stroke(r, rayon, Stroke::new(1.0, g.bord), StrokeKind::Inside);
}

/// Sphère brillante (bulle, pastille, bouton rond).
pub fn orbe(p: &egui::Painter, c: Pos2, rayon: f32, g: Gel) {
    let r = Rect::from_center_size(c, Vec2::splat(rayon * 2.0));
    p.add(vdeg(r, rayon, g.haut, g.bas));
    p.with_clip_rect(r).add(lueur(
        Rect::from_center_size(c + vec2(0.0, rayon * 0.55), vec2(rayon * 1.4, rayon * 0.9)),
        g.lueur.gamma_multiply(0.7),
        rayon * 0.6,
    ));
    let h = Rect::from_center_size(c - vec2(0.0, rayon * 0.42), vec2(rayon * 1.3, rayon * 0.95));
    p.add(vdeg(h, rayon * 0.6, blanc(235), blanc(20)));
    p.circle_stroke(c, rayon - 0.5, Stroke::new(1.0, g.bord));
}

/// Texte posé sur le ciel : blanc, ombre bleu nuit douce (lisible sur le fond).
/// `largeur` : retour à la ligne au-delà (`f32::INFINITY` = une seule ligne), lignes alignées
/// selon `ancre`.
pub fn texte_ciel(
    p: &egui::Painter,
    pos: Pos2,
    ancre: Align2,
    t: &str,
    police: FontId,
    largeur: f32,
) -> Rect {
    let mut job = egui::text::LayoutJob::simple(t.to_string(), police, Color32::WHITE, largeur);
    job.halign = ancre.x();
    let g = p.layout_job(job);
    let r = ancre.anchor_size(pos, g.size());
    // Lignes centrées / à droite : la galerie déborde à gauche de son origine.
    let r0 = r.min - g.rect.min.to_vec2();
    // Ombre portée + halo sombre tout autour (texte des icônes du bureau Vista).
    for (d, a) in [
        (vec2(0.0, 1.5), 110),
        (vec2(0.0, 0.5), 70),
        (vec2(-1.0, 0.5), 45),
        (vec2(1.0, 0.5), 45),
        (vec2(0.0, -0.8), 35),
        (vec2(0.0, 2.5), 40),
    ] {
        p.galley_with_override_text_color(r0 + d, g.clone(), nuit(a));
    }
    p.galley(r0, g, Color32::WHITE);
    r
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
    Aimant,
    Actualiser,
    Copier,
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
            // Trois curseurs : rail fin, bouton plein.
            for (y, x) in [(4.0, 10.5), (8.0, 5.5), (12.0, 9.0)] {
                trait_(p, &[q(2.0, y), q(14.0, y)], Stroke::new(1.3 * k, c));
                p.circle_filled(q(x, y), 2.4 * k, c);
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
        Icone::Aimant => aimant(p, q, s),
        Icone::Actualiser => {
            // Arc de 300° et sa pointe de flèche.
            let pts: Vec<Pos2> = (0..=20)
                .map(|i| {
                    let a = -1.2 + i as f32 * 0.25;
                    q(8.0 + 5.2 * a.cos(), 8.0 + 5.2 * a.sin())
                })
                .collect();
            trait_(p, &pts, s);
            p.add(Shape::convex_polygon(
                vec![q(11.0, 0.8), q(14.6, 4.6), q(9.6, 5.6)],
                c,
                Stroke::NONE,
            ));
        }
        Icone::Copier => {
            p.rect_stroke(
                Rect::from_min_max(q(5.5, 5.5), q(14.0, 14.5)),
                2.0 * k,
                s,
                StrokeKind::Middle,
            );
            trait_(
                p,
                &[q(10.5, 2.5), q(3.5, 2.5), q(2.5, 3.5), q(2.5, 11.0)],
                s,
            );
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

/// Aimant en U (liens magnet, torrents) : `q` place un point du carré de 16.
fn aimant(p: &egui::Painter, q: impl Fn(f32, f32) -> Pos2, s: Stroke) {
    let mut pts = vec![q(3.0, 2.5), q(3.0, 8.5)];
    pts.extend((1..12).map(|i| {
        let a = std::f32::consts::PI * (1.0 - i as f32 / 12.0);
        q(8.0 + 5.0 * a.cos(), 8.5 + 5.0 * a.sin())
    }));
    pts.extend([q(13.0, 8.5), q(13.0, 2.5)]);
    trait_(p, &pts, s);
    // Pôles : deux barrettes.
    for x in [3.0, 13.0] {
        p.line_segment([q(x - 2.0, 5.0), q(x + 2.0, 5.0)], s);
    }
}

/// Icône blanche avec une ombre d'un pixel (sur gel coloré).
pub fn icone_relief(p: &egui::Painter, r: Rect, i: Icone) {
    icone(p, r.translate(vec2(0.0, 1.0)), i, nuit(90));
    icone(p, r, i, Color32::WHITE);
}

/// Pastille d'icône : carré arrondi en gel coloré, pictogramme blanc (navigation, astuces).
pub fn tuile_icone(p: &egui::Painter, r: Rect, i: Icone, g: Gel) {
    gel(p, r, r.width() * 0.3, g, true, false);
    icone_relief(
        p,
        Rect::from_center_size(r.center(), Vec2::splat(r.width() * 0.58)),
        i,
    );
}

#[derive(Clone, Copy, PartialEq)]
pub enum TypeFichier {
    Archive,
    Video,
    Disque,
    Document,
    Torrent,
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
            "torrent" => TypeFichier::Torrent,
            _ => TypeFichier::Autre,
        }
    }

    fn couleur(self) -> Color32 {
        match self {
            TypeFichier::Archive => rgb(0xf0, 0x9c, 0x22),
            TypeFichier::Video => rgb(0xe8, 0x4a, 0x78),
            TypeFichier::Disque => rgb(0x12, 0xae, 0xa8),
            TypeFichier::Document => rgb(0x2a, 0x7c, 0xe0),
            TypeFichier::Torrent => rgb(0x2b, 0xa8, 0x5a),
            TypeFichier::Autre => rgb(0x6a, 0x6c, 0xe6),
        }
    }
}

/// Tuile de type de fichier : carré arrondi en gel, pictogramme blanc, reflet par-dessus.
pub fn tuile_fichier(p: &egui::Painter, r: Rect, t: TypeFichier) {
    let c = t.couleur();
    let rayon = r.width() * 0.26;
    gel(p, r, rayon, gel_de(c), true, false);
    let g = Rect::from_center_size(r.center(), Vec2::splat(r.width() * 0.52));
    let k = g.width() / 16.0;
    for (d, c) in [(vec2(0.0, 1.0), nuit(80)), (Vec2::ZERO, Color32::WHITE)] {
        let q = |x: f32, y: f32| g.min + d + vec2(x * k, y * k);
        let s = Stroke::new(1.6 * k.max(0.9), c);
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
                    Stroke::new(1.2 * k, c.gamma_multiply(0.8)),
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
            TypeFichier::Torrent => aimant(p, q, s),
            TypeFichier::Autre => page(p),
        }
    }
    // Second reflet léger par-dessus le pictogramme : l'effet « sous verre ».
    p.add(reflet(r.shrink2(vec2(1.5, 1.0)), rayon - 1.0, 0.25));
}

/// Pastille d'état : bulle claire teintée, petite sphère de couleur, texte foncé de la même
/// teinte. Renvoie le rectangle occupé.
pub fn pastille(ui: &Ui, gauche_centre: Pos2, t: &str, c: Color32) -> Rect {
    let ct = mix(c, TEXTE, 0.3);
    let g = ui.painter().layout_no_wrap(t.to_string(), demi(11.5), ct);
    let r = Rect::from_min_size(
        pos2(gauche_centre.x, gauche_centre.y - 10.0),
        vec2(g.size().x + 30.0, 20.0),
    );
    let p = ui.painter();
    p.add(vdeg(
        r,
        10.0,
        mix(Color32::WHITE, c, 0.08),
        mix(Color32::WHITE, c, 0.2),
    ));
    p.add(reflet(r.shrink(1.0), 9.0, 0.7));
    p.rect_stroke(
        r,
        10.0,
        Stroke::new(1.0, c.gamma_multiply(0.45)),
        StrokeKind::Inside,
    );
    orbe(p, pos2(r.left() + 11.0, r.center().y), 4.5, gel_de(c));
    p.galley(
        pos2(r.left() + 20.0, r.center().y - g.size().y / 2.0),
        g,
        ct,
    );
    r
}

/// Fond de survol animé (0 → 1) d'un élément interactif.
pub fn survol(ui: &Ui, r: &Response) -> f32 {
    ui.ctx()
        .animate_bool_with_time(r.id.with("survol"), r.hovered(), 0.12)
}

/// Sélection Windows 7 (ligne de liste choisie ou survolée) : dégradé bleu très clair,
/// liseré bleu, liseré blanc intérieur. `force` : 1 = sélectionné, < 1 = survol.
pub fn selection(p: &egui::Painter, r: Rect, rayon: f32, force: f32) {
    if force <= 0.0 {
        return;
    }
    let (haut, bas, bord) = (
        rgb(0xec, 0xf5, 0xfe),
        rgb(0xc3, 0xdf, 0xfb),
        rgb(0x7d, 0xa6, 0xd8),
    );
    let a = force.clamp(0.0, 1.0);
    p.add(vdeg(
        r,
        rayon,
        haut.gamma_multiply(a),
        bas.gamma_multiply(a),
    ));
    p.rect_stroke(
        r.shrink(1.0),
        rayon - 1.0,
        Stroke::new(1.0, blanc((170.0 * a) as u8)),
        StrokeKind::Inside,
    );
    p.rect_stroke(
        r,
        rayon,
        Stroke::new(1.0, bord.gamma_multiply(a)),
        StrokeKind::Inside,
    );
}

/// Bouton à icône à un emplacement donné (au-dessus d'une carte cliquable) : bulle argentée
/// au survol.
pub fn bouton_icone_ici(ui: &Ui, r: Rect, id: egui::Id, i: Icone, aide: &str) -> Response {
    let rep = ui.interact(r, id, Sense::click());
    let h = survol(ui, &rep);
    let p = ui.painter();
    if h > 0.0 {
        let b = r.shrink(1.0);
        let g = gel_mix(GEL_ARGENT, GEL_ARGENT_SURVOL, 0.6);
        let g = Gel {
            haut: g.haut.gamma_multiply(h),
            bas: g.bas.gamma_multiply(h),
            bord: g.bord.gamma_multiply(h),
            lueur: g.lueur.gamma_multiply(h),
        };
        gel(
            p,
            b,
            b.height() / 2.0,
            g,
            false,
            rep.is_pointer_button_down_on(),
        );
    }
    let c = mix(TEXTE2, TEXTE, h);
    icone(
        p,
        Rect::from_center_size(r.center(), Vec2::splat(14.0)),
        i,
        c,
    );
    anneau_focus(ui, &rep, r, r.height() / 2.0);
    rep.on_hover_text(aide)
}

/// Creux d'un champ : blanc, ombre intérieure en haut.
fn creux(p: &egui::Painter, r: Rect, rayon: f32) {
    p.rect_filled(r, rayon, blanc(238));
    p.add(vdeg(r, rayon, nuit(26), Color32::TRANSPARENT));
    let haut = Rect::from_min_size(r.min, vec2(r.width(), 5.0_f32.min(r.height())));
    let cr = CornerRadius {
        nw: rayon as u8,
        ne: rayon as u8,
        sw: 0,
        se: 0,
    };
    p.add(vdeg(haut, cr, nuit(22), Color32::TRANSPARENT));
}

/// Champ de saisie d'une ligne : creux blanc, icône facultative, halo bleu au focus.
pub fn champ(
    ui: &mut Ui,
    r: Rect,
    texte: &mut String,
    indice: &str,
    i: Option<Icone>,
    police: FontId,
) -> Response {
    let p = ui.painter().clone();
    let fond = p.add(Shape::Noop);
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
            .text_color(TEXTE)
            .font(police)
            .frame(egui::Frame::NONE.inner_margin(Margin::symmetric(0, marge)))
            .margin(Margin::symmetric(0, marge))
            .desired_width(zone.width()),
    );
    let rayon = 9.0;
    let mut v = Vec::new();
    let a = ui
        .ctx()
        .animate_bool_with_time(rep.id.with("focus"), rep.has_focus(), 0.15);
    if a > 0.0 {
        v.push(lueur(
            r.expand(3.0),
            rgb(0x5c, 0xc8, 0xff).gamma_multiply(0.55 * a),
            6.0,
        ));
    }
    v.push(Shape::rect_filled(r, rayon, blanc(240)));
    v.push(vdeg(r, rayon, nuit(18), Color32::TRANSPARENT));
    let bord = if rep.has_focus() {
        rgb(0x2f, 0x7f, 0xd8)
    } else if rep.hovered() {
        rgb(0x5f, 0x9e, 0xe0)
    } else {
        nuit(80)
    };
    v.push(Shape::rect_stroke(
        r,
        rayon,
        Stroke::new(1.0, bord),
        StrokeKind::Inside,
    ));
    p.set(fond, Shape::Vec(v));
    rep
}

/// Libellé centré d'un bouton (icône facultative) ; `relief` : texte blanc ombré.
fn etiquette(p: &egui::Painter, r: Rect, t: &str, i: Option<Icone>, relief: bool, c: Color32) {
    let g = p.layout_no_wrap(t.to_string(), demi(13.5), c);
    let li = match (i.is_some(), t.is_empty()) {
        (false, _) => 0.0,
        (true, true) => 16.0,
        (true, false) => 22.0,
    };
    let x = r.center().x - (g.size().x + li) / 2.0;
    if let Some(i) = i {
        let ri = Rect::from_min_size(pos2(x, r.center().y - 8.0), Vec2::splat(16.0));
        if relief {
            icone_relief(p, ri, i);
        } else {
            icone(p, ri, i, c);
        }
    }
    let pos = pos2(x + li, r.center().y - g.size().y / 2.0);
    if relief {
        p.galley_with_override_text_color(pos + vec2(0.0, 1.0), g.clone(), nuit(110));
    }
    p.galley(pos, g, c);
}

/// Bouton gel coloré (bleu ou vert) : halo de la couleur au survol, texte blanc en relief.
pub fn bouton_gel(ui: &mut Ui, t: &str, i: Option<Icone>, taille: Vec2, g: Gel) -> Response {
    let (r, rep) = ui.allocate_exact_size(taille, Sense::click());
    let h = survol(ui, &rep);
    let p = ui.painter();
    if h > 0.0 {
        p.add(lueur(r.expand(4.0), g.lueur.gamma_multiply(0.6 * h), 10.0));
    }
    let g = Gel {
        haut: mix(g.haut, Color32::WHITE, 0.12 * h),
        ..g
    };
    gel(
        p,
        r,
        r.height() / 2.0,
        g,
        true,
        rep.is_pointer_button_down_on(),
    );
    etiquette(p, r, t, i, true, Color32::WHITE);
    anneau_focus(ui, &rep, r, r.height() / 2.0);
    rep
}

/// Bouton principal : gel bleu.
pub fn bouton_principal(ui: &mut Ui, t: &str, i: Option<Icone>, taille: Vec2) -> Response {
    bouton_gel(ui, t, i, taille, GEL_BLEU)
}

/// Bouton secondaire : gel argenté, bleu clair au survol (Windows 7).
pub fn bouton_secondaire(ui: &mut Ui, t: &str, i: Option<Icone>, taille: Vec2) -> Response {
    let (r, rep) = ui.allocate_exact_size(taille, Sense::click());
    let h = survol(ui, &rep);
    let p = ui.painter();
    gel(
        p,
        r,
        r.height() / 2.0,
        gel_mix(GEL_ARGENT, GEL_ARGENT_SURVOL, h),
        true,
        rep.is_pointer_button_down_on(),
    );
    etiquette(p, r, t, i, false, TEXTE);
    anneau_focus(ui, &rep, r, r.height() / 2.0);
    rep
}

/// Anneau bleu autour d'un élément qui a le focus clavier.
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

/// Bouton rond brillant (curseur, interrupteur).
fn bouton_rond(p: &egui::Painter, c: Pos2, rayon: f32, h: f32) {
    p.circle_filled(c + vec2(0.0, 1.5), rayon + 0.5, nuit(45));
    let g = gel_mix(GEL_ARGENT, GEL_ARGENT_SURVOL, h);
    orbe(p, c, rayon, g);
}

/// Interrupteur animé (44 × 24) : creux argenté, gel bleu allumé, bouton rond brillant.
pub fn interrupteur(ui: &mut Ui, on: &mut bool) -> Response {
    let (r, mut rep) = ui.allocate_exact_size(vec2(44.0, 24.0), Sense::click());
    if rep.clicked() {
        *on = !*on;
        rep.mark_changed();
    }
    let a = ui.ctx().animate_bool_with_time(rep.id, *on, 0.16);
    let h = survol(ui, &rep);
    let p = ui.painter();
    creux(p, r, 12.0);
    if a > 0.0 {
        let g = GEL_BLEU;
        let g = Gel {
            haut: g.haut.gamma_multiply(a),
            bas: g.bas.gamma_multiply(a),
            bord: g.bord.gamma_multiply(a),
            lueur: g.lueur.gamma_multiply(a),
        };
        gel(p, r, 12.0, g, false, false);
    }
    p.rect_stroke(
        r,
        12.0,
        Stroke::new(1.0, nuit((70.0 * (1.0 - a)) as u8)),
        StrokeKind::Inside,
    );
    let x = egui::lerp(r.left() + 12.0..=r.right() - 12.0, a);
    bouton_rond(p, pos2(x, r.center().y), 10.0, h);
    anneau_focus(ui, &rep, r, 12.0);
    rep
}

/// Contrôle segmenté : gel argenté, segment choisi en gel bleu qui glisse.
pub fn segmente<T: PartialEq + Copy>(ui: &mut Ui, v: &mut T, options: &[(T, &str)]) -> Response {
    let l = 96.0;
    let (r, mut rep) =
        ui.allocate_exact_size(vec2(l * options.len() as f32 + 6.0, 34.0), Sense::hover());
    let p = ui.painter().clone();
    gel(&p, r, 17.0, GEL_ARGENT, true, false);
    let i = options.iter().position(|o| o.0 == *v).unwrap_or(0);
    let x = ui
        .ctx()
        .animate_value_with_time(rep.id.with("glisse"), i as f32, 0.18);
    let sel = Rect::from_min_size(pos2(r.left() + 3.0 + x * l, r.top() + 3.0), vec2(l, 28.0));
    gel(&p, sel, 14.0, GEL_BLEU, false, false);
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
        if k == i {
            let g = p.layout_no_wrap(nom.to_string(), demi(13.0), Color32::WHITE);
            let pos = cr.center() - g.size() / 2.0;
            p.galley_with_override_text_color(pos + vec2(0.0, 1.0), g.clone(), nuit(110));
            p.galley(pos, g, Color32::WHITE);
        } else {
            let col = if c.hovered() { TITRE } else { TEXTE2 };
            p.text(cr.center(), Align2::CENTER_CENTER, *nom, moyen(13.0), col);
        }
    }
    rep
}

/// Reflet qui balaie une jauge en cours (barre de progression Vista). Ne redessine que pendant
/// le passage (12 i/s) ; entre deux passages, une seule image est demandée pour le suivant.
fn balayage(p: &egui::Painter, f: Rect) {
    const PERIODE: f64 = 2.6;
    const PASSAGE: f64 = 1.3;
    let ctx = p.ctx();
    let t = ctx.input(|i| i.time);
    let phase = t % PERIODE;
    if phase < PASSAGE {
        let k = (phase / PASSAGE) as f32;
        let bande = (f.width() * 0.5).clamp(24.0, 110.0);
        let x = f.left() - bande + (f.width() + bande * 2.0) * k;
        let b = Rect::from_center_size(pos2(x, f.center().y), vec2(bande, f.height() * 1.6));
        p.with_clip_rect(f.intersect(p.clip_rect()))
            .add(lueur(b, blanc(150), bande * 0.45));
        // 12 i/s : assez pour le reflet, l'interface reste sous ~12 images/s en téléchargement.
        ctx.request_repaint_after(std::time::Duration::from_millis(80));
    } else {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(PERIODE - phase));
    }
}

/// Jauge brillante : creux argenté + remplissage gel de la couleur `c` ; `en_cours` : reflet
/// qui balaie la partie remplie.
pub fn jauge(p: &egui::Painter, r: Rect, frac: f32, c: Color32, en_cours: bool) {
    let rayon = r.height() / 2.0;
    creux(p, r, rayon);
    p.rect_stroke(r, rayon, Stroke::new(1.0, nuit(55)), StrokeKind::Inside);
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
    gel(p, f, f.height() / 2.0, gel_de(c), false, false);
    if en_cours && f.width() > 8.0 {
        balayage(p, f);
    }
}

/// Petite bulle brillante qui marque le début d'un morceau volé.
pub fn bulle_vol(p: &egui::Painter, c: Pos2, couleur: Color32) {
    orbe(p, c, 4.5, gel_de(couleur));
}

/// Carte du fichier entier : un morceau par segment (fait = gel de la couleur de sa
/// connexion, reste = la même teinte en transparence), bulle au début de chaque morceau volé.
pub fn carte_fichier(ui: &mut Ui, l: &Live, hauteur: f32) {
    let (zone, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), hauteur + 10.0), Sense::hover());
    let r = Rect::from_min_max(pos2(zone.left(), zone.top() + 10.0), zone.max);
    let p = ui.painter();
    creux(p, r.expand(1.0), 5.0);
    let p = &p.with_clip_rect(r.intersect(p.clip_rect()));
    let taille = l.size.unwrap_or(0).max(1) as f32;
    let x = |o: u64| r.left() + r.width() * (o as f32 / taille).min(1.0);
    let n = l.workers.len();
    let mut bulles = Vec::new();
    let pb = ui.painter();
    for s in &l.segments {
        let c = s.worker.map_or(TEXTE3, |w| couleur_connexion(w, n));
        let fin = s.end.min(l.size.unwrap_or(s.end));
        let (x0, x1) = (x(s.start), x(fin));
        if x1 - x0 < 0.5 {
            continue;
        }
        let tout = Rect::from_x_y_ranges(x0..=x1, r.y_range());
        p.rect_filled(tout, 0.0, c.gamma_multiply(0.18));
        let xp = x(s.pos).min(tout.right());
        if xp > tout.left() {
            let fait = Rect::from_x_y_ranges(tout.left()..=xp, r.y_range());
            let c = if s.remaining() == 0 {
                mix(c, Color32::WHITE, 0.25)
            } else {
                c
            };
            p.add(vdeg(fait, 0.0, mix(c, Color32::WHITE, 0.35), c));
            // Front de téléchargement : liseré clair.
            p.vline(xp, r.y_range(), Stroke::new(1.0, blanc(200)));
        }
        if x0 > r.left() + 1.0 {
            p.vline(x0, r.y_range(), Stroke::new(1.0, blanc(230)));
        }
        if s.stolen {
            bulles.push((pos2(x0, r.top() - 7.0), c));
        }
    }
    // Un seul reflet sur toute la barre : un tube de verre rempli de couleurs.
    p.add(reflet(r, 4.0, 0.75));
    pb.rect_stroke(
        r.expand(1.0),
        5.0,
        Stroke::new(1.0, nuit(70)),
        StrokeKind::Inside,
    );
    for (b, c) in bulles {
        bulle_vol(pb, b, c);
    }
}

/// Carte des pièces d'un torrent : une case par groupe de pièces, gel de la palette des
/// connexions (selon la position) d'autant plus plein que le groupe est reçu.
pub fn carte_pieces(ui: &mut Ui, cases: &[u8], hauteur: f32) {
    let (zone, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), hauteur + 10.0), Sense::hover());
    let r = Rect::from_min_max(pos2(zone.left(), zone.top() + 10.0), zone.max);
    let p = ui.painter();
    creux(p, r.expand(1.0), 5.0);
    let n = cases.len().max(1) as f32;
    let pc = &p.with_clip_rect(r.intersect(p.clip_rect()));
    for (i, &v) in cases.iter().enumerate() {
        let (x0, x1) = (
            r.left() + r.width() * i as f32 / n,
            r.left() + r.width() * (i + 1) as f32 / n,
        );
        let c = teinte(i as f32 / n);
        let case = Rect::from_x_y_ranges(x0..=x1, r.y_range());
        if v == 255 {
            pc.add(vdeg(case, 0.0, mix(c, Color32::WHITE, 0.35), c));
        } else if v > 0 {
            pc.rect_filled(case, 0.0, c.gamma_multiply(0.18 + 0.5 * v as f32 / 255.0));
        } else {
            pc.rect_filled(case, 0.0, c.gamma_multiply(0.1));
        }
    }
    pc.add(reflet(r, 4.0, 0.75));
    p.rect_stroke(
        r.expand(1.0),
        5.0,
        Stroke::new(1.0, nuit(70)),
        StrokeKind::Inside,
    );
}

/// Case à cocher brillante (choix des fichiers d'un torrent).
pub fn coche(ui: &mut Ui, on: &mut bool, t: &str, droite: &str) -> Response {
    let w = ui.available_width();
    let (r, mut rep) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
    if rep.clicked() {
        *on = !*on;
        rep.mark_changed();
    }
    let h = survol(ui, &rep);
    let p = ui.painter();
    selection(p, r, 6.0, 0.45 * h);
    let b = Rect::from_center_size(pos2(r.left() + 14.0, r.center().y), Vec2::splat(16.0));
    if *on {
        gel(p, b, 4.0, GEL_BLEU, false, false);
        trait_(
            p,
            &[
                b.min + vec2(3.8, 8.3),
                b.min + vec2(6.8, 11.3),
                b.min + vec2(12.2, 4.8),
            ],
            Stroke::new(2.0, Color32::WHITE),
        );
    } else {
        creux(p, b, 4.0);
        p.rect_stroke(b, 4.0, Stroke::new(1.0, nuit(90)), StrokeKind::Inside);
    }
    let dw = p
        .layout_no_wrap(droite.to_string(), regulier(12.5), TEXTE3)
        .size()
        .x;
    let g = ligne(ui, t, regulier(13.0), TEXTE, r.width() - 40.0 - dw - 12.0);
    p.galley(
        pos2(r.left() + 30.0, r.center().y - g.size().y / 2.0),
        g,
        TEXTE,
    );
    texte(
        ui,
        r.right_center() - vec2(8.0, 0.0),
        Align2::RIGHT_CENTER,
        droite,
        regulier(12.5),
        TEXTE3,
    );
    rep
}

/// Une ligne par connexion : numéro, jauge du morceau courant, mention « volé », débit.
/// `erreurs` : (« essais, dernière erreur : », aide d'une connexion retirée).
pub fn connexions(
    ui: &mut Ui,
    l: &Live,
    vitesses: &[f64],
    vole: &str,
    aide_vol: &str,
    erreurs: (&str, &str),
) {
    let large = ui.available_width();
    let n = l.workers.len();
    let bv = ui
        .painter()
        .layout_no_wrap(vole.to_string(), demi(10.5), TEXTE)
        .size()
        .x
        + 22.0;
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
        let droite = r.right() - 82.0;
        let barre = Rect::from_x_y_ranges(
            r.left() + 28.0..=droite - if vole_ici { bv + 8.0 } else { 0.0 },
            r.center().y - 3.5..=r.center().y + 3.5,
        );
        let frac = ui
            .ctx()
            .animate_value_with_time(rep.id.with("frac"), frac, 0.45);
        let cj = if actif {
            c
        } else {
            mix(c, Color32::WHITE, 0.55)
        };
        jauge(p, barre, frac, cj, false);
        if vole_ici {
            // Bulle claire, sphère de la couleur de la connexion, texte foncé lisible.
            let b = Rect::from_min_size(pos2(droite - bv, r.center().y - 8.0), vec2(bv, 16.0));
            p.add(vdeg(
                b,
                8.0,
                mix(Color32::WHITE, c, 0.1),
                mix(Color32::WHITE, c, 0.28),
            ));
            p.add(reflet(b.shrink(1.0), 7.0, 0.8));
            p.rect_stroke(
                b,
                8.0,
                Stroke::new(1.0, mix(c, Color32::BLACK, 0.2).gamma_multiply(0.7)),
                StrokeKind::Inside,
            );
            bulle_vol(p, pos2(b.left() + 8.0, b.center().y), c);
            p.text(
                pos2(b.left() + 15.0, b.center().y),
                Align2::LEFT_CENTER,
                vole,
                demi(10.5),
                mix(c, TEXTE, 0.55),
            );
            ui.interact(b, rep.id.with("vol"), Sense::hover())
                .on_hover_text(aide_vol);
        }
        let v = vitesses.get(w).copied().unwrap_or(0.0);
        // Retirée par l'hôte (403, 429…) : sa raison à la place du débit.
        let (texte_d, cd) = match &st.error {
            Some(e) if st.dropped => (raison_courte(e), DANGER),
            _ if actif => (format!("{}/s", super::octets(v as u64)), TEXTE2),
            _ => ("—".into(), TEXTE3),
        };
        ui.painter().text(
            pos2(r.right(), r.center().y),
            Align2::RIGHT_CENTER,
            texte_d,
            regulier(12.0),
            cd,
        );
        if let Some(e) = &st.error {
            let aide = format!("{} {} {e}", st.retries, erreurs.0);
            rep.on_hover_text(if st.dropped {
                format!("{}\n{aide}", erreurs.1)
            } else {
                aide
            });
        }
    }
}

/// « HTTP 403 » → « 403 », « bad Content-Range: … » → « range », « timeout » tel quel.
fn raison_courte(e: &str) -> String {
    if let Some(c) = e.strip_prefix("HTTP ") {
        return c.split(' ').next().unwrap_or(c).to_string();
    }
    if e.starts_with("bad Content-Range") {
        return "range".into();
    }
    e.split(' ').next().unwrap_or(e).to_string()
}

/// Courbe du débit total : vitre creuse, aire vert d'eau → bleu translucide, ligne brillante,
/// bulle au dernier point.
pub fn courbe(ui: &mut Ui, h: &[f32], max: f32, hauteur: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), hauteur), Sense::hover());
    let p = ui.painter();
    p.add(vdeg(r, 10.0, blanc(60), blanc(150)));
    p.rect_stroke(r, 10.0, Stroke::new(1.0, nuit(45)), StrokeKind::Inside);
    let r = r.shrink2(vec2(10.0, 8.0));
    for k in 1..4 {
        let y = r.top() + r.height() * k as f32 / 4.0;
        p.hline(r.x_range(), y.round() + 0.5, Stroke::new(1.0, nuit(18)));
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
    let coul = |x: f32| teinte(0.15 + 0.65 * ((x - r.left()) / r.width()));
    let mut m = Mesh::default();
    for q in &pts {
        let c = coul(q.x);
        m.colored_vertex(*q, c.gamma_multiply(0.62));
        m.colored_vertex(pos2(q.x, r.bottom()), c.gamma_multiply(0.08));
    }
    for i in 0..pts.len() as u32 - 1 {
        let a = i * 2;
        m.add_triangle(a, a + 1, a + 2);
        m.add_triangle(a + 1, a + 2, a + 3);
    }
    p.add(Shape::mesh(m));
    let gauche = r.left();
    let large = r.width();
    // Liseré blanc juste au-dessus de la ligne : arête de verre.
    p.add(Shape::line(
        pts.iter().map(|q| *q - vec2(0.0, 1.5)).collect(),
        Stroke::new(1.5, blanc(170)),
    ));
    p.add(Shape::line(
        pts.clone(),
        egui::epaint::PathStroke::new_uv(2.5, move |_, q| {
            mix(
                teinte(0.15 + 0.65 * ((q.x - gauche) / large)),
                Color32::BLACK,
                0.12,
            )
        }),
    ));
    if let Some(d) = pts.last() {
        let c = coul(d.x);
        p.add(lueur(
            Rect::from_center_size(*d, Vec2::splat(18.0)),
            c.gamma_multiply(0.35),
            6.0,
        ));
        orbe(p, *d, 5.5, gel_de(c));
    }
}

/// Curseur : rail creux, remplissage en gel aqua, bouton rond brillant. Flèches du clavier
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
    let rail = r.shrink2(vec2(10.0, 0.0));
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
    let piste = Rect::from_x_y_ranges(rail.x_range(), r.center().y - 3.5..=r.center().y + 3.5);
    let x = rail.left() + rail.width() * t;
    jauge(
        p,
        piste,
        (x - piste.left()) / piste.width(),
        teinte(0.55),
        false,
    );
    let h = ui
        .ctx()
        .animate_bool_with_time(rep.id, rep.hovered() || rep.dragged(), 0.12);
    let c = pos2(x, r.center().y);
    if h > 0.0 {
        p.add(lueur(
            Rect::from_center_size(c, Vec2::splat(30.0)),
            rgb(0x5c, 0xc8, 0xff).gamma_multiply(0.45 * h),
            10.0,
        ));
    }
    bouton_rond(p, c, 9.5, h);
    if rep.has_focus() {
        p.circle_stroke(c, 12.5, Stroke::new(1.5, ACCENT));
    }
    rep
}

/// Petite boîte de valeur (texte) à côté d'un curseur.
pub fn boite_valeur(ui: &mut Ui, t: &str, largeur: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(largeur, 30.0), Sense::hover());
    let p = ui.painter();
    creux(p, r, 8.0);
    p.rect_stroke(r, 8.0, Stroke::new(1.0, nuit(70)), StrokeKind::Inside);
    p.text(r.center(), Align2::CENTER_CENTER, t, demi(13.0), TEXTE);
}
