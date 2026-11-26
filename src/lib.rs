//! Cœur de l'appli : moteur de téléchargement (sans UI), réutilisé par l'exe et les tests.

/// Nom de l'appli (provisoire) : le seul endroit à changer avec le paquet Cargo.
pub const APP_NAME: &str = "fluxdl";

pub mod engine;
