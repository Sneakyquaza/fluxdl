// Icône de l'appli dessinée par calcul (aucune image embarquée) : carré arrondi au dégradé
// de la palette, trois flux parallèles qui convergent vers une flèche. Utilisé par la fenêtre
// (main.rs) et inclus par build.rs pour l'icône de l'exe Windows : pas de `//!` ici.

/// Pixels RGBA (non prémultipliés) d'une icône carrée de `n` px.
pub fn icone_rgba(n: u32) -> Vec<u8> {
    // Arrêts du dégradé diagonal : aqua, bleu, violet (mêmes teintes que l'interface).
    const ARRETS: [(f32, [f32; 3]); 3] = [
        (0.0, [46.0, 211.0, 198.0]),
        (0.5, [79.0, 125.0, 255.0]),
        (1.0, [154.0, 92.0, 255.0]),
    ];
    let couleur = |t: f32| {
        let t = t.clamp(0.0, 1.0);
        let (i, j) = if t < 0.5 { (0, 1) } else { (1, 2) };
        let k = (t - ARRETS[i].0) / (ARRETS[j].0 - ARRETS[i].0);
        let (a, b) = (ARRETS[i].1, ARRETS[j].1);
        [0, 1, 2].map(|c| a[c] + (b[c] - a[c]) * k)
    };
    // Distance signée à un segment épais (capsule) et à un carré arrondi, en unités 0..1.
    let capsule = |x: f32, y: f32, a: (f32, f32), b: (f32, f32), r: f32| {
        let (px, py, bx, by) = (x - a.0, y - a.1, b.0 - a.0, b.1 - a.1);
        let h = ((px * bx + py * by) / (bx * bx + by * by)).clamp(0.0, 1.0);
        ((px - bx * h).powi(2) + (py - by * h).powi(2)).sqrt() - r
    };
    let carre = |x: f32, y: f32| {
        let (m, r) = (0.04, 0.23);
        let qx = (x - 0.5).abs() - (0.5 - m - r);
        let qy = (y - 0.5).abs() - (0.5 - m - r);
        (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r
    };
    let e = 0.047;
    // Segments épais : x0, y0, x1, y1, opacité du blanc.
    let traits: [[f32; 5]; 6] = [
        [0.5, 0.2, 0.5, 0.64, 1.0],
        [0.29, 0.25, 0.37, 0.41, 0.62],
        [0.71, 0.25, 0.63, 0.41, 0.62],
        [0.3, 0.5, 0.5, 0.7, 1.0],
        [0.7, 0.5, 0.5, 0.7, 1.0],
        [0.31, 0.8, 0.69, 0.8, 1.0],
    ];
    let mut px = vec![0u8; (n * n * 4) as usize];
    let ss = 4;
    for py in 0..n {
        for pxl in 0..n {
            let (mut fond, mut rgb) = (0.0f32, [0.0f32; 3]);
            for sy in 0..ss {
                for sx in 0..ss {
                    let x = (pxl as f32 + (sx as f32 + 0.5) / ss as f32) / n as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / ss as f32) / n as f32;
                    if carre(x, y) > 0.0 {
                        continue;
                    }
                    fond += 1.0;
                    let c = couleur((x * 0.55 + y * 0.45 - 0.05) / 0.9);
                    // Léger voile clair en haut : relief discret.
                    let voile = (0.5 - y).max(0.0) * 0.18;
                    let v = c.map(|c| c + (255.0 - c) * voile);
                    let mut a = 0.0f32;
                    for [x0, y0, x1, y1, o] in traits {
                        if capsule(x, y, (x0, y0), (x1, y1), e) <= 0.0 {
                            a = a.max(o);
                        }
                    }
                    for (s, v) in rgb.iter_mut().zip(v) {
                        *s += v + (255.0 - v) * a;
                    }
                }
            }
            let i = ((py * n + pxl) * 4) as usize;
            if fond > 0.0 {
                for (d, s) in px[i..i + 3].iter_mut().zip(rgb) {
                    *d = (s / fond).round() as u8;
                }
                px[i + 3] = (255.0 * fond / (ss * ss) as f32).round() as u8;
            }
        }
    }
    px
}
