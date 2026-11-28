//! Limite de débit : seau à jetons partagé par des connexions (et des téléchargements).
//!
//! Chaque lecture prend ses octets dans le seau ; s'il est vide, la connexion attend que le
//! débit permis l'ait rempli (dette remboursée). Débit changé en direct par `regler` ; débit 0
//! (illimité) : un seul chargement atomique par lecture, ni verrou ni attente.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Rafale permise après un temps sans lecture : un quart de seconde au débit permis.
const RAFALE_S: f64 = 0.25;

pub struct Seau {
    /// Octets par seconde (0 = illimité).
    debit: AtomicU64,
    /// Jetons (négatif = dette des connexions qui attendent) et instant du dernier remplissage.
    etat: Mutex<(f64, Instant)>,
}

impl Default for Seau {
    fn default() -> Self {
        Seau::new(0)
    }
}

impl Seau {
    pub fn new(debit: u64) -> Self {
        Seau {
            debit: AtomicU64::new(debit),
            etat: Mutex::new((0.0, Instant::now())),
        }
    }

    pub fn debit(&self) -> u64 {
        self.debit.load(Ordering::Relaxed)
    }

    /// Nouveau débit (0 = illimité), pris en compte à la lecture suivante ; la dette due à
    /// l'ancien débit est oubliée.
    pub fn regler(&self, debit: u64) {
        if self.debit.swap(debit, Ordering::Relaxed) != debit {
            *self.etat.lock().unwrap_or_else(|e| e.into_inner()) = (0.0, Instant::now());
        }
    }

    /// Prend `n` octets reçus ; attente avant la lecture suivante (`None` : tout de suite).
    pub fn prendre(&self, n: u64) -> Option<Duration> {
        let d = self.debit();
        if d == 0 {
            return None;
        }
        let d = d as f64;
        let mut e = self.etat.lock().unwrap_or_else(|e| e.into_inner());
        let maintenant = Instant::now();
        let jetons = e.0 + maintenant.duration_since(e.1).as_secs_f64() * d;
        *e = (jetons.min(d * RAFALE_S) - n as f64, maintenant);
        (e.0 < 0.0).then(|| Duration::from_secs_f64(-e.0 / d))
    }
}
