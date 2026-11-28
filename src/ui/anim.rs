//! Animations douces (esprit Vista / 7) : easings, durées (0 si « Réduire les animations »),
//! valeurs animées et impulsions uniques. Une image n'est demandée QUE pendant un mouvement :
//! au repos, rien (0 % CPU).

use eframe::egui::{Context, Id};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static REDUIT: AtomicBool = AtomicBool::new(false);

/// « Réduire les animations » (réglage, ou animations coupées dans le système, lu une fois) :
/// toutes les durées à 0. Rend l'état retenu.
pub fn reduire(oui: bool) -> bool {
    static SYSTEME: OnceLock<bool> = OnceLock::new();
    let oui = oui || *SYSTEME.get_or_init(crate::systeme::animations_coupees);
    REDUIT.store(oui, Ordering::Relaxed);
    oui
}

/// Ralenti pour les captures (`FLUXDL_ANIM_LENT=20` : 20 fois plus lent), lu une fois.
fn lent() -> f32 {
    static LENT: OnceLock<f32> = OnceLock::new();
    *LENT.get_or_init(|| {
        std::env::var("FLUXDL_ANIM_LENT")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|v| *v > 0.0)
            .unwrap_or(1.0)
    })
}

/// Durée réelle d'une animation de `s` secondes.
pub fn duree(s: f32) -> f32 {
    if REDUIT.load(Ordering::Relaxed) {
        0.0
    } else {
        s * lent()
    }
}

/// Sortie cubique : part vite, se pose doucement.
pub fn sortie_cubique(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    1.0 - u * u * u
}

/// Sortie quintique : plus vive au départ, arrivée très douce.
pub fn sortie_quintique(t: f32) -> f32 {
    let u = 1.0 - t.clamp(0.0, 1.0);
    1.0 - u * u * u * u * u
}

/// Ressort amorti léger : dépasse d'environ 3 % puis se pose (exactement 1 à la fin).
pub fn ressort(t: f32) -> f32 {
    let f = |t: f32| 1.0 - (-11.0 * t).exp() * (8.0 * t).cos();
    f(t.clamp(0.0, 1.0)) / f(1.0)
}

/// Avance `v` vers `cible` (0 ou 1) de `dt / duree`.
pub fn avancer(v: f32, cible: f32, dt: f32, duree: f32) -> f32 {
    if duree <= 0.0 {
        return cible;
    }
    let pas = dt.max(0.0) / duree;
    if cible > v {
        (v + pas).min(cible)
    } else {
        (v - pas).max(cible)
    }
}

/// Interpolation adoucie de `de` à `vers`, `t` secondes après le départ.
pub fn interpoler(de: f32, vers: f32, t: f32, duree: f32) -> f32 {
    if duree <= 0.0 {
        return vers;
    }
    de + (vers - de) * sortie_cubique(t / duree)
}

#[derive(Clone, Copy)]
struct Etat {
    v: f32,
    t: f64,
}

/// Progression linéaire 0 → 1 d'un état vrai/faux, durées différentes à la montée et à la
/// descente (secondes, avant `duree`). Demande une image tant que ça bouge.
pub fn progres(ctx: &Context, id: Id, on: bool, monte: f32, descend: f32) -> f32 {
    let now = ctx.input(|i| i.time);
    let cible = if on { 1.0 } else { 0.0 };
    let e = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(id, || Etat { v: cible, t: now }));
    let d = duree(if on { monte } else { descend });
    let v = avancer(e.v, cible, (now - e.t) as f32, d);
    ctx.data_mut(|d| d.insert_temp(id, Etat { v, t: now }));
    if v != cible {
        ctx.request_repaint();
    }
    v
}

/// Comme `progres`, adouci (sortie cubique dans les deux sens).
pub fn bool(ctx: &Context, id: Id, on: bool, monte: f32, descend: f32) -> f32 {
    let v = progres(ctx, id, on, monte, descend);
    if on {
        sortie_cubique(v)
    } else {
        1.0 - sortie_cubique(1.0 - v)
    }
}

#[derive(Clone, Copy)]
struct Valeur {
    de: f32,
    vers: f32,
    depuis: f64,
    d: f32,
}

/// Valeur qui glisse vers `cible` en `s` secondes (sortie cubique), sans saut si la cible
/// change en route.
pub fn valeur(ctx: &Context, id: Id, cible: f32, s: f32) -> f32 {
    let now = ctx.input(|i| i.time);
    let mut e = ctx.data_mut(|d| {
        *d.get_temp_mut_or_insert_with(id, || Valeur {
            de: cible,
            vers: cible,
            depuis: now,
            d: 0.0,
        })
    });
    let v = interpoler(e.de, e.vers, (now - e.depuis) as f32, e.d);
    if e.vers != cible {
        e = Valeur {
            de: v,
            vers: cible,
            depuis: now,
            d: duree(s),
        };
        ctx.data_mut(|d| d.insert_temp(id, e));
        // Départ : la valeur est encore celle d'avant.
        ctx.request_repaint();
        return v;
    }
    if v != cible {
        ctx.request_repaint();
    }
    v
}

#[derive(Clone, Copy)]
struct Impulsion {
    actif: bool,
    passe: u64,
    debut: Option<f64>,
}

/// Effet unique de `s` secondes (reflet qui balaie, éclat) : part quand `actif` passe de faux à
/// vrai, ou à l'apparition si `a_l_ouverture` (élément absent de l'image précédente). Rend la
/// progression 0..1 pendant l'effet, `None` sinon. Jamais en boucle.
pub fn impulsion(ctx: &Context, id: Id, actif: bool, s: f32, a_l_ouverture: bool) -> Option<f32> {
    let now = ctx.input(|i| i.time);
    let passe = ctx.cumulative_pass_nr();
    let avant = ctx.data_mut(|d| d.get_temp::<Impulsion>(id));
    let neuf = avant.is_none_or(|a| passe > a.passe + 2);
    let debut = match avant {
        _ if neuf => (actif && a_l_ouverture).then_some(now),
        Some(a) if actif && !a.actif => Some(now),
        Some(a) => a.debut,
        None => None,
    };
    ctx.data_mut(|d| {
        d.insert_temp(
            id,
            Impulsion {
                actif,
                passe,
                debut,
            },
        )
    });
    let d = duree(s);
    let k = ((now - debut?) as f32 / d.max(f32::EPSILON)).max(0.0);
    if d <= 0.0 || k >= 1.0 {
        return None;
    }
    ctx.request_repaint();
    Some(k)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easings_bornes_et_monotonie() {
        for f in [sortie_cubique, sortie_quintique] {
            assert_eq!(f(0.0), 0.0);
            assert_eq!(f(1.0), 1.0);
            assert_eq!(f(-1.0), 0.0);
            assert_eq!(f(2.0), 1.0);
            let mut avant = 0.0;
            for k in 1..=100 {
                let v = f(k as f32 / 100.0);
                assert!(v >= avant);
                avant = v;
            }
            // Sortie : déjà plus de la moitié du chemin au quart du temps.
            assert!(f(0.25) > 0.5);
        }
        assert!(sortie_quintique(0.2) > sortie_cubique(0.2));
    }

    #[test]
    fn ressort_leger() {
        assert!(ressort(0.0).abs() < 1e-6);
        assert!((ressort(1.0) - 1.0).abs() < 1e-6);
        let pic = (0..=100)
            .map(|k| ressort(k as f32 / 100.0))
            .fold(0.0, f32::max);
        // Dépassement léger, pas de rebond caricatural.
        assert!(pic > 1.0 && pic < 1.05, "pic {pic}");
    }

    #[test]
    fn avancer_et_interpoler() {
        assert_eq!(avancer(0.0, 1.0, 0.1, 0.2), 0.5);
        assert_eq!(avancer(0.5, 1.0, 1.0, 0.2), 1.0);
        assert_eq!(avancer(1.0, 0.0, 0.05, 0.1), 0.5);
        assert_eq!(avancer(0.3, 0.0, 0.0, 0.0), 0.0);
        assert_eq!(avancer(0.3, 1.0, -1.0, 0.2), 0.3);
        assert_eq!(interpoler(2.0, 4.0, 0.0, 0.3), 2.0);
        assert_eq!(interpoler(2.0, 4.0, 0.3, 0.3), 4.0);
        assert_eq!(interpoler(2.0, 4.0, 9.0, 0.3), 4.0);
        assert_eq!(interpoler(2.0, 4.0, 0.0, 0.0), 4.0);
        let m = interpoler(0.0, 10.0, 0.15, 0.3);
        assert!(m > 5.0 && m < 10.0);
    }
}
