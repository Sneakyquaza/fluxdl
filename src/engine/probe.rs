//! Sonde : une requête `Range: bytes=0-0` donne taille, prise en charge des plages, validateurs
//! (ETag / Last-Modified) et nom de fichier, sans télécharger le corps.

use super::Probe;

pub fn probe(
    agent: &ureq::Agent,
    url: &str,
    headers: &[(String, String)],
) -> Result<Probe, String> {
    let mut req = agent.get(url).set("Range", "bytes=0-0");
    for (k, v) in headers {
        req = req.set(k, v);
    }
    let resp = match req.call() {
        // Page de connexion ou anti-robot (Cloudflare…) : c'est une page web, pas le fichier.
        Err(ureq::Error::Status(c, r))
            if matches!(c, 401 | 403 | 429 | 503) && super::est_html(r.header("Content-Type")) =>
        {
            return Err(super::ERR_PAGE.into())
        }
        r => r.map_err(|e| format!("probe: {e}"))?,
    };
    let entete = |n: &str| resp.header(n).map(str::to_string);
    let (size, ranges) = if resp.status() == 206 {
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
    let file_name = entete("Content-Disposition")
        .and_then(|c| name_from_disposition(&c))
        .or_else(|| name_from_url(resp.get_url()))
        .map(|n| sanitize(&n))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "download".into());
    // Le corps (fichier entier si 200) n'est pas lu : la connexion est fermée en lâchant `resp`.
    Ok(Probe {
        size,
        ranges,
        etag: entete("ETag"),
        last_modified: entete("Last-Modified"),
        file_name,
        content_type: entete("Content-Type"),
    })
}

/// `filename*=UTF-8''nom%20encodé` (prioritaire) ou `filename="nom"`.
pub fn name_from_disposition(c: &str) -> Option<String> {
    let mut simple = None;
    for part in c.split(';').map(str::trim) {
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        let v = v.trim().trim_matches('"');
        match k.trim().to_ascii_lowercase().as_str() {
            "filename*" => {
                let v = v.rsplit('\'').next().unwrap_or(v);
                return Some(percent_decode(v));
            }
            "filename" => simple = Some(v.to_string()),
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

/// Nom sûr sur Windows et Linux : pas de séparateur, caractère réservé ni contrôle.
pub fn sanitize(n: &str) -> String {
    let n: String = n
        .chars()
        .map(|c| {
            if c.is_control() || r#"/\:*?"<>|"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    n.trim().trim_matches('.').trim().to_string()
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
}
