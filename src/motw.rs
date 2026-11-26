//! Mark-of-the-Web (Windows) : flux `:Zone.Identifier` posé sur chaque fichier téléchargé, comme
//! le font les navigateurs. SmartScreen et le mode protégé d'Office traitent alors le fichier
//! comme venu d'Internet. Ailleurs (ou sur FAT/exFAT, sans flux de données alternatifs) : rien.

use std::path::Path;

/// Contenu du marqueur : zone Internet (3), page d'origine et adresse du fichier, sans
/// identifiants, paramètres (jetons des liens signés) ni fragment.
pub fn contenu(url: &str, referrer: Option<&str>) -> String {
    let mut s = String::from("[ZoneTransfer]\r\nZoneId=3\r\n");
    if let Some(r) = referrer.and_then(propre) {
        s += &format!("ReferrerUrl={r}\r\n");
    }
    s += &format!(
        "HostUrl={}\r\n",
        propre(url).unwrap_or_else(|| "about:internet".into())
    );
    s
}

/// `http(s)://hôte/chemin` sans `user:pass@`, `?…` ni `#…` ; autre chose : `None`.
fn propre(u: &str) -> Option<String> {
    let (schema, reste) = u.split_once("://")?;
    if !matches!(schema.to_ascii_lowercase().as_str(), "http" | "https") {
        return None;
    }
    let reste = reste.split(['?', '#']).next().unwrap_or("");
    let (hote, chemin) = reste.split_at(reste.find('/').unwrap_or(reste.len()));
    let hote = hote.rsplit('@').next().unwrap_or("");
    (!hote.is_empty() && !u.chars().any(char::is_control))
        .then(|| format!("{schema}://{hote}{chemin}"))
}

/// Marque `p` (un fichier, ou tous les fichiers d'un dossier de torrent).
pub fn marquer(p: &Path, url: &str, referrer: Option<&str>) {
    if !cfg!(windows) {
        return;
    }
    let c = contenu(url, referrer);
    let mut pile = vec![p.to_path_buf()];
    let mut n = 0;
    while let Some(f) = pile.pop() {
        // raccourci : 10 000 fichiers au plus (torrent géant) ; au-delà, non marqués.
        n += 1;
        if n > 10_000 {
            return;
        }
        if f.is_dir() {
            if let Ok(l) = std::fs::read_dir(&f) {
                pile.extend(l.filter_map(|e| e.ok().map(|e| e.path())));
            }
        } else if f.is_file() {
            let mut ads = f.into_os_string();
            ads.push(":Zone.Identifier");
            let _ = std::fs::write(ads, &c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marqueur_sans_secret() {
        let c = contenu(
            "https://user:pw@cdn.example/f/setup.exe?token=SECRET#x",
            Some("https://site.example/page?session=abc"),
        );
        assert_eq!(
            c,
            "[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=https://site.example/page\r\nHostUrl=https://cdn.example/f/setup.exe\r\n"
        );
        assert!(!c.contains("SECRET") && !c.contains("pw"));
        // Lien magnet / référent invalide : zone Internet quand même.
        assert_eq!(
            contenu("magnet:?xt=urn:btih:abc", Some("javascript:x")),
            "[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=about:internet\r\n"
        );
    }
}
