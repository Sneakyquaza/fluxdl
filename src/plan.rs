//! Planificateur de la file (heure de démarrage, heure d'arrêt) et action de fin : fonctions
//! pures, l'heure locale vient de `systeme::minute_locale` (minutes depuis minuit).

use crate::store::Status;

const JOUR: u16 = 1440;

/// Minutes pour aller de `de` à `a` en avançant (passage de minuit compris).
fn ecart(de: u16, a: u16) -> u16 {
    (a % JOUR + JOUR - de % JOUR) % JOUR
}

/// L'heure `h` est passée entre la minute vue avant (`vu`, exclue) et `m` (comprise).
fn franchi(vu: u16, m: u16, h: u16) -> bool {
    (1..=ecart(vu, m)).contains(&ecart(vu, h))
}

/// `m` est dans la plage de marche : de `debut` à `fin` (exclue) ; sans fin, la minute du
/// démarrage seulement (démarrage unique).
pub fn dans_plage(m: u16, debut: u16, fin: Option<u16>) -> bool {
    match fin {
        Some(f) => ecart(debut, m) < ecart(debut, f),
        None => m == debut,
    }
}

/// File en attente (`arme`) à l'ouverture ou quand l'heure change : hors de la plage de marche.
pub fn arme_au_depart(m: u16, debut: Option<u16>, fin: Option<u16>) -> bool {
    debut.is_some_and(|d| !dans_plage(m, d, fin))
}

/// Minute `m` (vue avant : `vu`) : (file en attente, mettre en pause ce qui tourne).
/// L'heure de démarrage passée libère la file ; l'heure d'arrêt la remet en attente jusqu'au
/// prochain démarrage. Les deux passées depuis `vu` (appli endormie) : la dernière compte.
pub fn planifier(
    arme: bool,
    vu: u16,
    m: u16,
    debut: Option<u16>,
    fin: Option<u16>,
) -> (bool, bool) {
    let Some(d) = debut else {
        return (false, false);
    };
    let demarre = franchi(vu, m, d).then(|| ecart(vu, d));
    let arrete = fin.filter(|&f| franchi(vu, m, f)).map(|f| ecart(vu, f));
    match (demarre, arrete) {
        (None, None) => (arme, false),
        (Some(a), Some(b)) if a > b => (false, false),
        (Some(_), None) => (false, false),
        (_, Some(_)) => (true, !arme),
    }
}

/// Action de fin à lancer : au moins un téléchargement fini pendant cette session (jamais au
/// lancement de l'appli), et tous finis ou annulés (aucun en attente, en cours, en pause ni en
/// échec).
pub fn fin_atteinte<'a>(statuts: impl IntoIterator<Item = &'a Status>, fini_session: bool) -> bool {
    fini_session
        && statuts
            .into_iter()
            .all(|s| matches!(s, Status::Done | Status::Canceled))
}

/// « 02:00 ».
pub fn hhmm(m: u16) -> String {
    format!("{:02}:{:02}", m / 60 % 24, m % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hm(h: u16, m: u16) -> u16 {
        h * 60 + m
    }

    #[test]
    fn demarrage_et_arret() {
        let (d, f) = (Some(hm(2, 0)), Some(hm(6, 30)));
        // Lancée à 23:00 pour 02:00 : en attente ; à 01:59 rien, à 02:00 la file démarre.
        assert!(arme_au_depart(hm(23, 0), d, None));
        assert!(arme_au_depart(hm(23, 0), d, f));
        assert_eq!(planifier(true, hm(1, 58), hm(1, 59), d, f), (true, false));
        assert_eq!(planifier(true, hm(1, 59), hm(2, 0), d, f), (false, false));
        // Dans la plage au lancement : tout de suite ; sans heure d'arrêt, seulement à l'heure.
        assert!(!arme_au_depart(hm(3, 0), d, f));
        assert!(arme_au_depart(hm(3, 0), d, None));
        assert!(!arme_au_depart(hm(2, 0), d, None));
        // 06:30 : pause et attente du lendemain 02:00.
        assert_eq!(planifier(false, hm(6, 29), hm(6, 30), d, f), (true, true));
        assert_eq!(planifier(true, hm(6, 30), hm(12, 0), d, f), (true, false));
        // Sans planification : jamais en attente.
        assert_eq!(
            planifier(true, hm(1, 59), hm(2, 0), None, f),
            (false, false)
        );
        assert!(!arme_au_depart(hm(1, 0), None, None));
    }

    #[test]
    fn passage_de_minuit() {
        // Démarrage 23:30, arrêt 01:00 : la plage passe minuit.
        let (d, f) = (Some(hm(23, 30)), Some(hm(1, 0)));
        assert!(!arme_au_depart(hm(0, 15), d, f));
        assert!(arme_au_depart(hm(1, 0), d, f));
        assert!(arme_au_depart(hm(23, 0), d, f));
        assert_eq!(planifier(false, hm(0, 59), hm(1, 0), d, f), (true, true));
        // Démarrage à 00:00 vu de 23:59, ou franchi pendant un trou (appli endormie).
        let minuit = Some(0);
        assert_eq!(planifier(true, hm(23, 59), 0, minuit, None), (false, false));
        assert_eq!(
            planifier(true, hm(22, 0), hm(3, 0), minuit, None),
            (false, false)
        );
        // Les deux heures franchies pendant le trou : la dernière compte.
        let (d2, f2) = (Some(hm(22, 0)), Some(hm(6, 0)));
        assert_eq!(planifier(true, hm(5, 0), hm(23, 0), d2, f2), (false, false));
        assert_eq!(planifier(true, hm(21, 0), hm(7, 0), d2, f2), (true, false));
        // Même minute : rien ne change.
        assert_eq!(
            planifier(true, hm(2, 0), hm(2, 0), Some(hm(2, 0)), None),
            (true, false)
        );
        assert_eq!(hhmm(hm(2, 5)), "02:05");
    }

    #[test]
    fn action_de_fin_seulement_si_tout_a_reussi() {
        let fini = [Status::Done, Status::Canceled, Status::Done];
        assert!(fin_atteinte(&fini, true));
        // Au lancement (rien fini pendant cette session) : jamais.
        assert!(!fin_atteinte(&fini, false));
        for autre in [
            Status::Failed("HTTP 404".into()),
            Status::Paused,
            Status::Queued,
            Status::Running,
        ] {
            assert!(
                !fin_atteinte(&[Status::Done, autre.clone()], true),
                "{autre:?}"
            );
        }
    }
}
