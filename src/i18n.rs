//! Traductions : la clé est le texte anglais, la table donne le français.

use crate::store::Lang;

const FR: &[(&str, &str)] = &[
    ("Done early? Take half.", "Fini tôt ? Prends la moitié."),
    ("Downloads", "Téléchargements"),
    ("Benchmark", "Comparatif"),
    ("Settings", "Réglages"),
    ("Paste a link (http/https)…", "Colle un lien (http/https)…"),
    ("Download", "Télécharger"),
    ("Folder", "Dossier"),
    ("optional", "facultatif"),
    ("Connections", "Connexions"),
    ("connections", "connexions"),
    ("No downloads yet", "Aucun téléchargement"),
    ("Cancel", "Annuler"),
    ("Pause", "Pause"),
    ("Resume", "Reprendre"),
    ("QUEUED", "EN ATTENTE"),
    ("LIVE", "EN COURS"),
    ("PAUSED", "EN PAUSE"),
    ("DONE", "FINI"),
    ("ERROR", "ERREUR"),
    ("CANCELED", "ANNULÉ"),
    ("No live data: resume to see the connections.", "Pas de données en direct : reprends pour voir les connexions."),
    ("CONNECTIONS", "CONNEXIONS"),
    ("active", "actives"),
    ("steals (done early? take half)", "vols (fini tôt ? prends la moitié)"),
    ("remote file changed: restarted", "fichier distant changé : repris de zéro"),
    ("server refuses ranges: 1 connection", "le serveur refuse les plages : 1 connexion"),
    ("File map", "Carte du fichier"),
    ("STOLEN", "VOLÉ"),
    ("CONNECTION", "CONNEXION"),
    ("Total speed", "Débit total"),
    ("BENCHMARK", "COMPARATIF"),
    (
        "Downloads the same link with 1 connection, then with N, to a temporary file deleted afterwards.",
        "Télécharge le même lien avec 1 connexion puis avec N, dans un fichier temporaire effacé ensuite.",
    ),
    ("Max seconds per run", "Secondes max par essai"),
    ("Run benchmark", "Lancer le comparatif"),
    ("Stop", "Arrêter"),
    ("Measuring", "Mesure avec"),
    (
        "Little gain: the server or your line is probably already saturated by one connection.",
        "Peu de gain : le serveur ou ta ligne est sans doute déjà saturé par une seule connexion.",
    ),
    ("SETTINGS", "RÉGLAGES"),
    ("Connections per download", "Connexions par téléchargement"),
    ("Default folder", "Dossier par défaut"),
    ("Speed limit (MB/s, 0 = none)", "Limite de débit (Mo/s, 0 = aucune)"),
    ("Language", "Langue"),
    (
        "Up to 32 connections per file. Unfinished downloads resume where they stopped, even after a crash.",
        "Jusqu'à 32 connexions par fichier. Les téléchargements inachevés reprennent où ils se sont arrêtés, même après un plantage.",
    ),
];

pub fn t(lang: Lang, en: &'static str) -> &'static str {
    match lang {
        Lang::En => en,
        Lang::Fr => FR.iter().find(|(k, _)| *k == en).map_or(en, |(_, f)| f),
    }
}
