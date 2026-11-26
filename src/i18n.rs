//! Traductions : la clé est le texte anglais, la table donne le français.

use crate::store::Lang;

const FR: &[(&str, &str)] = &[
    ("Done early? Take half.", "Fini tôt ? Prends la moitié."),
    (
        "A connection that finishes early takes half of the largest remaining range.",
        "Une connexion qui finit tôt prend la moitié du plus gros morceau restant.",
    ),
    ("Version", "Version"),
    ("Downloads", "Téléchargements"),
    ("Benchmark", "Comparatif"),
    ("Settings", "Réglages"),
    ("Paste a link…", "Colle un lien…"),
    ("Paste", "Coller"),
    ("Download", "Télécharger"),
    ("Choose folder", "Choisir le dossier"),
    ("Connections", "Connexions"),
    ("connections", "connexions"),
    ("Connections per download", "Connexions par téléchargement"),
    (
        "Optional checksum, verified after download.",
        "Somme de contrôle facultative, vérifiée après le téléchargement.",
    ),
    ("No downloads yet", "Aucun téléchargement"),
    (
        "Paste a link above. Each file is split across up to 32 parallel connections.",
        "Colle un lien ci-dessus. Chaque fichier est réparti sur jusqu'à 32 connexions en parallèle.",
    ),
    ("Pause", "Pause"),
    ("Resume", "Reprendre"),
    ("Cancel", "Annuler"),
    ("Open folder", "Ouvrir le dossier"),
    ("Remove from list", "Retirer de la liste"),
    ("Queued", "En attente"),
    ("Downloading", "En cours"),
    ("Paused", "En pause"),
    ("Completed", "Terminé"),
    ("Failed", "Échec"),
    ("Canceled", "Annulé"),
    ("left", "restant"),
    ("active", "actives"),
    ("Nothing selected", "Aucune sélection"),
    (
        "Pick a download to watch its connections at work.",
        "Choisis un téléchargement pour voir ses connexions travailler.",
    ),
    ("Download complete.", "Téléchargement terminé."),
    (
        "No live data: resume to see the connections.",
        "Pas de données en direct : reprends pour voir les connexions.",
    ),
    (
        "Remote file changed: restarted from zero.",
        "Le fichier distant a changé : repris de zéro.",
    ),
    (
        "Server refuses ranges: 1 connection.",
        "Le serveur refuse les plages : 1 connexion.",
    ),
    ("Speed", "Débit"),
    ("Progress", "Avancement"),
    ("Time left", "Reste"),
    ("steals", "vols"),
    ("Interface", "Interface"),
    ("About", "À propos"),
    ("of", "sur"),
    ("File map", "Carte du fichier"),
    ("stolen range", "morceau volé"),
    ("stolen", "volé"),
    (
        "Done early? Take half. This connection finished its own range and took half of the largest remaining one.",
        "Fini tôt ? Prends la moitié. Cette connexion a fini son morceau et a pris la moitié du plus gros restant.",
    ),
    ("peak", "pic"),
    (
        "Downloads the same link with 1 connection, then with N, to a temporary file deleted afterwards.",
        "Télécharge le même lien avec 1 connexion puis avec N, dans un fichier temporaire effacé ensuite.",
    ),
    ("Max seconds per run", "Secondes max par essai"),
    ("Run benchmark", "Lancer le comparatif"),
    ("Stop", "Arrêter"),
    ("Measuring with", "Mesure avec"),
    ("faster", "plus rapide"),
    ("with", "avec"),
    (
        "Little gain: the server or your line is probably already saturated by one connection.",
        "Peu de gain : le serveur ou ta ligne est sans doute déjà saturé par une seule connexion.",
    ),
    (
        "More connections help on fast lines and with slow servers.",
        "Plus de connexions aident sur une ligne rapide et avec un serveur lent.",
    ),
    ("Default folder", "Dossier par défaut"),
    ("New downloads are saved here.", "Les nouveaux téléchargements sont enregistrés ici."),
    ("Browse", "Parcourir"),
    ("Speed limit", "Limite de débit"),
    (
        "Per download. Off means no limit.",
        "Par téléchargement. Désactivée : aucune limite.",
    ),
    ("Language", "Langue"),
    ("Applies immediately.", "S'applique tout de suite."),
    ("MIT license, free and open source.", "licence MIT, libre et gratuit."),
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
