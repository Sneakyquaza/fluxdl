// Icône de l'appli dessinée par calcul (aucune image embarquée) : sphère de verre bleu ciel →
// aqua → vert (style Aero), flèche blanche vers le bas, lueur claire en bas, reflet en haut.
// Utilisé par la fenêtre (main.rs) et inclus par build.rs pour l'icône de l'exe Windows :
// pas de `//!` ici.

/// Pixels RGBA (non prémultipliés) d'une icône carrée de `n` px.
pub fn icone_rgba(n: u32) -> Vec<u8> {
    // Arrêts du dégradé vertical de la sphère : bleu ciel, aqua, vert frais.
    const ARRETS: [(f32, [f32; 3]); 3] = [
        (0.0, [36.0, 140.0, 236.0]),
        (0.55, [22.0, 182.0, 214.0]),
        (1.0, [70.0, 196.0, 52.0]),
    ];
    let couleur = |t: f32| {
        let t = t.clamp(0.0, 1.0);
        let (i, j) = if t < ARRETS[1].0 { (0, 1) } else { (1, 2) };
        let k = (t - ARRETS[i].0) / (ARRETS[j].0 - ARRETS[i].0);
        let (a, b) = (ARRETS[i].1, ARRETS[j].1);
        [0, 1, 2].map(|c| a[c] + (b[c] - a[c]) * k)
    };
    let melange = |a: [f32; 3], b: [f32; 3], t: f32| [0, 1, 2].map(|c| a[c] + (b[c] - a[c]) * t);
    // Flèche pleine : tige + pointe triangulaire (coordonnées 0..1).
    let fleche = |x: f32, y: f32| {
        let tige = (x - 0.5).abs() < 0.085 && (0.2..0.52).contains(&y);
        // Pointe : triangle (0.26, 0.47) (0.74, 0.47) (0.5, 0.77).
        let pointe = (0.47..=0.77).contains(&y) && (x - 0.5).abs() <= 0.24 * (0.77 - y) / 0.3;
        tige || pointe
    };
    let (rayon, bord) = (0.46f32, 0.018f32);
    let mut px = vec![0u8; (n * n * 4) as usize];
    let ss = 4;
    for py in 0..n {
        for pxl in 0..n {
            let (mut fond, mut rgb) = (0.0f32, [0.0f32; 3]);
            for sy in 0..ss {
                for sx in 0..ss {
                    let x = (pxl as f32 + (sx as f32 + 0.5) / ss as f32) / n as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / ss as f32) / n as f32;
                    let d = ((x - 0.5).powi(2) + (y - 0.5).powi(2)).sqrt();
                    if d > rayon {
                        continue;
                    }
                    fond += 1.0;
                    let mut c = couleur((y - 0.04) / 0.92);
                    // Volume : bord assombri.
                    let v = 1.0 - 0.28 * (d / rayon).powi(3);
                    c = c.map(|c| c * v);
                    // Lueur claire en bas (lumière qui traverse le verre).
                    let q = ((x - 0.5) / 0.3).powi(2) + ((y - 0.84) / 0.14).powi(2);
                    if q < 1.0 {
                        c = melange(c, [210.0, 255.0, 160.0], 0.6 * (1.0 - q).powi(2));
                    }
                    // Ombre douce de la flèche, puis la flèche (blanc → bleu très pâle).
                    if fleche(x - 0.012, y - 0.022) {
                        c = melange(c, [8.0, 60.0, 110.0], 0.35);
                    }
                    if fleche(x, y) {
                        c = melange(
                            [255.0; 3],
                            [215.0, 240.0, 255.0],
                            ((y - 0.2) / 0.57).max(0.0),
                        );
                    }
                    // Reflet : ellipse claire en haut, forte en haut, qui s'efface.
                    let q = ((x - 0.5) / 0.34).powi(2) + ((y - 0.29) / 0.22).powi(2);
                    if q < 1.0 {
                        let a =
                            (0.85 - 0.75 * ((y - 0.07) / 0.44).clamp(0.0, 1.0)) * (1.0 - q.powi(4));
                        c = melange(c, [255.0; 3], a);
                    }
                    // Liseré bleu foncé.
                    if d > rayon - bord {
                        c = melange(c, [10.0, 70.0, 140.0], 0.65);
                    }
                    for (s, v) in rgb.iter_mut().zip(c) {
                        *s += v;
                    }
                }
            }
            let i = ((py * n + pxl) * 4) as usize;
            if fond > 0.0 {
                for (d, s) in px[i..i + 3].iter_mut().zip(rgb) {
                    *d = (s / fond).round().clamp(0.0, 255.0) as u8;
                }
                px[i + 3] = (255.0 * fond / (ss * ss) as f32).round() as u8;
            }
        }
    }
    px
}
