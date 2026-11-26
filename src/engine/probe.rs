//! Sonde : une requête `Range: bytes=0-0` donne taille, prise en charge des plages, validateurs
//! (ETag / Last-Modified) et nom de fichier, sans télécharger le corps.

use super::Probe;

pub fn probe(
    agent: &ureq::Agent,
    url: &str,
    headers: &[(String, String)],
) -> Result<Probe, String> {
    let (resp, url_finale) = match super::requete(agent, url, url, headers, Some("bytes=0-0")) {
        // Page de connexion ou anti-robot (Cloudflare…) : c'est une page web, pas le fichier.
        Err(ureq::Error::Status(c, r))
            if matches!(c, 401 | 403 | 429 | 503) && super::est_html(r.header("Content-Type")) =>
        {
            return Err(super::ERR_PAGE.into())
        }
        // Fichier vide : « 416, Content-Range: bytes */0 ».
        Err(ureq::Error::Status(416, r))
            if r.header("Content-Range").map(str::trim) == Some("bytes */0") =>
        {
            let url_finale = r.get_url().to_string();
            return Ok(Probe {
                size: Some(0),
                file_name: nom(None, &url_finale),
                url: url_finale,
                ..Default::default()
            });
        }
        // Message court, sans l'URL (jeton d'un lien signé).
        Err(e) => return Err(format!("probe: {}", super::raison(&e))),
        Ok(r) => r,
    };
    if (300..400).contains(&resp.status()) {
        return Err("probe: too many redirects".into());
    }
    let entete = |n: &str| resp.header(n).map(str::to_string);
    // Corps compressé malgré `Accept-Encoding: identity` : ni la taille ni les plages ne valent
    // pour le fichier ; la connexion unique échouera clairement (`ERR_ENCODAGE`).
    let encode = super::encode(&resp);
    let (size, ranges) = if encode {
        (None, false)
    } else if resp.status() == 206 {
        // « bytes 0-0/12345 » ; « */ » inconnu : pas de découpage possible.
        let total = entete("Content-Range").and_then(|c| {
            c.rsplit('/')
                .next()
                .and_then(|t| t.trim().parse::<u64>().ok())
        });
        (total, total.is_some())
    } else {
        (
            entete("Content-Length").and_then(|l| l.trim().parse().ok()),
            false,
        )
    };
    let file_name = nom(entete("Content-Disposition"), &url_finale);
    // Le corps (fichier entier si 200) n'est pas lu : la connexion est fermée en lâchant `resp`.
    Ok(Probe {
        size,
        ranges,
        etag: entete("ETag"),
        last_modified: entete("Last-Modified"),
        file_name,
        content_type: entete("Content-Type"),
        url: url_finale,
    })
}

/// Nom du fichier : Content-Disposition, sinon fin de l'URL, nettoyé ; « download » à défaut.
fn nom(disposition: Option<String>, url: &str) -> String {
    disposition
        .and_then(|c| name_from_disposition(&c))
        .or_else(|| name_from_url(url))
        .map(|n| sanitize(&n))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "download".into())
}

/// `filename*=UTF-8''nom%20encodé` (prioritaire) ou `filename="nom"` (RFC 6266 : chaîne entre
/// guillemets qui peut contenir `;` et `\"`).
pub fn name_from_disposition(c: &str) -> Option<String> {
    let mut simple = None;
    let mut reste = c;
    while !reste.is_empty() {
        // Nom du paramètre jusqu'à `=` ou `;`.
        let fin = reste.find(['=', ';']).unwrap_or(reste.len());
        let k = reste[..fin].trim().to_ascii_lowercase();
        reste = &reste[fin..];
        let mut v = String::new();
        if let Some(r) = reste.strip_prefix('=') {
            let r = r.trim_start();
            if let Some(mut q) = r.strip_prefix('"') {
                // Chaîne entre guillemets, `\x` = x.
                while let Some(ch) = q.chars().next() {
                    q = &q[ch.len_utf8()..];
                    match ch {
                        '"' => break,
                        '\\' => {
                            if let Some(e) = q.chars().next() {
                                v.push(e);
                                q = &q[e.len_utf8()..];
                            }
                        }
                        _ => v.push(ch),
                    }
                }
                reste = q;
            } else {
                let fin = r.find(';').unwrap_or(r.len());
                v = r[..fin].trim().to_string();
                reste = &r[fin..];
            }
        }
        reste = reste.find(';').map_or("", |i| &reste[i + 1..]);
        match k.as_str() {
            "filename*" => {
                // charset'langue'valeur%20encodée
                let v = v.splitn(3, '\'').nth(2).unwrap_or(&v);
                return Some(percent_decode(v));
            }
            "filename" => simple = Some(v),
            _ => {}
        }
    }
    simple
}

pub fn name_from_url(url: &str) -> Option<String> {
    let chemin = url.split(['?', '#']).next()?;
    let apres_hote = chemin.split_once("://").map_or(chemin, |(_, r)| r);
    let (_, chemin) = apres_hote.split_once('/')?;
    chemin
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .map(percent_decode)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Caractères de format invisibles (Unicode Cf : contrôles bidi, espaces sans chasse…) : un
/// `U+202E` fait lire « fdp.exe » comme « exe.pdf ».
fn invisible(c: char) -> bool {
    matches!(c,
        '\u{AD}' | '\u{600}'..='\u{605}' | '\u{61C}' | '\u{6DD}' | '\u{70F}' | '\u{180E}'
        | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{206F}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}'
        | '\u{E0001}' | '\u{E0020}'..='\u{E007F}')
}

/// Noms de périphériques Windows (avec ou sans extension : `NUL.zip` ouvre NUL).
fn reserve_windows(n: &str) -> bool {
    let base = n
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && base.as_bytes()[3].is_ascii_digit()
            && base.as_bytes()[3] != b'0')
}

/// Longueur maximale d'un nom (octets UTF-8) : place pour « (12) » et « .part.json ».
const NOM_MAX: usize = 200;

/// Nom sûr sur Windows et Linux : pas de séparateur, caractère réservé, contrôle ni caractère
/// invisible ; nom de périphérique préfixé ; 200 octets au plus, extension gardée.
pub fn sanitize(n: &str) -> String {
    let n: String = n
        .chars()
        .filter(|&c| !invisible(c))
        .map(|c| {
            if c.is_control() || r#"/\:*?"<>|"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let mut n = n.trim().trim_matches('.').trim().to_string();
    if reserve_windows(&n) {
        n.insert(0, '_');
    }
    if n.len() > NOM_MAX {
        let ext = match n.rsplit_once('.') {
            Some((b, e)) if !b.is_empty() && e.len() <= 16 => format!(".{e}"),
            _ => String::new(),
        };
        let mut coupe = NOM_MAX - ext.len();
        while !n.is_char_boundary(coupe) {
            coupe -= 1;
        }
        n = format!("{}{ext}", n[..coupe].trim_end());
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noms() {
        assert_eq!(
            name_from_disposition(r#"attachment; filename="a b.zip""#).as_deref(),
            Some("a b.zip")
        );
        assert_eq!(
            name_from_disposition("attachment; filename=x.bin; filename*=UTF-8''%C3%A9t%C3%A9.txt")
                .as_deref(),
            Some("été.txt")
        );
        assert_eq!(
            name_from_url("https://h.org/d/f%20g.iso?x=1").as_deref(),
            Some("f g.iso")
        );
        assert_eq!(name_from_url("https://h.org/"), None);
        assert_eq!(name_from_url("https://h.org"), None);
        assert_eq!(sanitize("../a:b?.txt"), "_a_b_.txt");
        assert_eq!(percent_decode("100%"), "100%");
    }

    #[test]
    fn disposition_rfc6266() {
        assert_eq!(
            name_from_disposition(r#"attachment; filename="rapport;2024.pdf""#).as_deref(),
            Some("rapport;2024.pdf")
        );
        assert_eq!(
            name_from_disposition(r#"attachment; filename="a \"b\".txt"; size=3"#).as_deref(),
            Some(r#"a "b".txt"#)
        );
        // filename* prioritaire même placé après.
        assert_eq!(
            name_from_disposition(
                r#"inline; filename="x;y.bin"; filename*=utf-8'fr'%E2%82%AC.bin"#
            )
            .as_deref(),
            Some("€.bin")
        );
        assert_eq!(name_from_disposition("attachment"), None);
    }

    #[test]
    fn noms_dangereux() {
        assert_eq!(sanitize("NUL.zip"), "_NUL.zip");
        assert_eq!(sanitize("con"), "_con");
        assert_eq!(sanitize("Com1.tar.gz"), "_Com1.tar.gz");
        assert_eq!(sanitize("lpt9"), "_lpt9");
        assert_eq!(sanitize("COM0.txt"), "COM0.txt");
        assert_eq!(sanitize("console.txt"), "console.txt");
        assert_eq!(sanitize("\u{202E}fdp.exe"), "fdp.exe");
        assert_eq!(sanitize("a\u{200B}b\u{2066}.pdf"), "ab.pdf");
        let long = format!("{}.zip", "é".repeat(200));
        let s = sanitize(&long);
        assert!(s.len() <= NOM_MAX, "{}", s.len());
        assert!(s.ends_with("é.zip"));
        assert_eq!(sanitize(&"x".repeat(300)).len(), NOM_MAX);
    }
}
