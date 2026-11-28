//! Cookies de connexion envoyés par l'extension pour une vidéo (abonnés, membres) : gardés en
//! mémoire seulement (jamais dans queue.json ni dans un journal), écrits au format Netscape dans
//! un fichier temporaire 0600 (`<données>/cookies/`) le temps d'un appel à yt-dlp
//! (`--cookies`), effacé dès que yt-dlp a fini ; ce qui reste après un plantage est effacé au
//! lancement suivant (`purger`).

use serde::Deserialize;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Cookies acceptés au plus par vidéo (un compte YouTube en a ~40).
pub const MAX: usize = 300;

/// Un cookie tel que l'extension le lit (`chrome.cookies`). `expiry` : secondes Unix, 0 = session.
#[derive(Clone, PartialEq, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    #[serde(default = "racine")]
    pub path: String,
    #[serde(default)]
    pub secure: bool,
    #[serde(default, rename = "httpOnly")]
    pub http_only: bool,
    #[serde(default)]
    pub expiry: f64,
}

fn racine() -> String {
    "/".into()
}

/// Jamais la valeur dans un `{:?}` (journal, panique).
impl std::fmt::Debug for Cookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cookie({} @ {}, ***)", self.name, self.domain)
    }
}

/// Hôte d'une URL (minuscules, sans port).
fn hote(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or("")
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .rsplit('@')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Les deux derniers libellés (`www.youtube.com` → `youtube.com`) ; youtu.be = youtube.com.
/// raccourci : pas de liste des suffixes publics (`co.uk`…) ; suffit pour les sites vidéo connus.
fn site(h: &str) -> String {
    let h = h.trim_start_matches('.').trim_end_matches('.');
    let l: Vec<&str> = h.rsplit('.').take(2).collect();
    let s = l.into_iter().rev().collect::<Vec<_>>().join(".");
    if s == "youtu.be" {
        "youtube.com".into()
    } else {
        s
    }
}

/// Site d'une page (`https://youtu.be/x` → `youtube.com`).
pub fn site_de(url: &str) -> String {
    site(&hote(url))
}

/// Cookie utilisable : son domaine est l'hôte de la page, un domaine parent qui le couvre
/// (`.youtube.com` pour `www.youtube.com`) ou un sous-domaine de l'hôte (même règle stricte que
/// `sameSite` de l'extension : `b.co.uk` ou `y.github.io` jamais pour `a.co.uk` / `x.github.io`) ;
/// sans tabulation ni retour à la ligne (le format Netscape n'a pas d'échappement : un tel cookie
/// est écarté, pas réparé). Parent : au moins deux libellés (`.com`, `.uk` refusés) ;
/// raccourci : sans liste des suffixes publics, un cookie `.co.uk` passerait (le navigateur n'en
/// crée jamais), à revoir si l'appli reçoit des cookies d'ailleurs que du navigateur.
fn valide(c: &Cookie, page: &str) -> bool {
    let propre = |s: &str| !s.chars().any(|c| c.is_control());
    let d = c.domain.trim_start_matches('.');
    !c.name.is_empty()
        && !d.is_empty()
        && !c.name.starts_with('#')
        && [&c.name, &c.value, &c.domain, &c.path]
            .iter()
            .all(|s| propre(s))
        && c.path.starts_with('/')
        && c.expiry.is_finite()
        && (d == page
            || (d.contains('.') && page.ends_with(&format!(".{d}")))
            || d.ends_with(&format!(".{page}")))
}

/// Cookies gardés pour la page `url` (autres sites et cookies illisibles écartés, `MAX` au plus).
pub fn pour_la_page(url: &str, v: Vec<Cookie>) -> Vec<Cookie> {
    let h = hote(url);
    let h = h.trim_end_matches('.');
    // youtu.be (liens courts) : les cookies sont ceux de youtube.com.
    let page = if h == "youtu.be" || h.ends_with(".youtu.be") {
        "youtube.com"
    } else {
        h
    };
    v.into_iter()
        .filter(|c| valide(c, page))
        .take(MAX)
        .collect()
}

/// Fichier au format Netscape (celui de `--cookies`) : en-tête exigé par yt-dlp, puis
/// `domaine  sous-domaines  chemin  https  expiration  nom  valeur` séparés par des tabulations.
/// `#HttpOnly_` devant le domaine d'un cookie httpOnly ; « sous-domaines » = domaine à point
/// initial (Python l'exige).
pub fn netscape(v: &[Cookie]) -> String {
    let mut s = String::from("# Netscape HTTP Cookie File\n\n");
    for c in v {
        let vf = |b: bool| if b { "TRUE" } else { "FALSE" };
        s.push_str(&format!(
            "{}{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            if c.http_only { "#HttpOnly_" } else { "" },
            c.domain,
            vf(c.domain.starts_with('.')),
            c.path,
            vf(c.secure),
            c.expiry.max(0.0) as u64,
            c.name,
            c.value
        ));
    }
    s
}

pub fn dossier() -> PathBuf {
    crate::store::data_dir().join("cookies")
}

/// Fichier de cookies temporaire, effacé quand il sort de portée.
pub struct Fichier(PathBuf);

impl Fichier {
    /// Écrit `v` dans un nouveau fichier de `d` (nom aléatoire, 0600 dès la création sous Unix ;
    /// Windows : dossier de données du profil, privé à l'utilisateur).
    pub fn creer(d: &Path, v: &[Cookie]) -> std::io::Result<Fichier> {
        let mut b = std::fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
        b.create(d)?;
        let f = d.join(format!("{}.txt", crate::serveur::nouveau_jeton()));
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let mut h = o.open(&f)?;
        let g = Fichier(f);
        h.write_all(netscape(v).as_bytes())?;
        Ok(g)
    }

    pub fn chemin(&self) -> &Path {
        &self.0
    }
}

impl Drop for Fichier {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Au lancement (instance unique déjà prise) : fichiers laissés par un plantage.
pub fn purger(d: &Path) {
    for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
        let _ = std::fs::remove_file(e.path());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &str, value: &str, domain: &str) -> Cookie {
        Cookie {
            name: name.into(),
            value: value.into(),
            domain: domain.into(),
            path: "/".into(),
            secure: true,
            http_only: false,
            expiry: 1_900_000_000.7,
        }
    }

    #[test]
    fn format_netscape() {
        let mut a = c("SAPISID", "a b;c=\"d\"", ".youtube.com");
        a.http_only = true;
        let mut b = c("auth-token", "x", "www.twitch.tv");
        (b.secure, b.expiry, b.path) = (false, 0.0, "/videos".into());
        assert_eq!(
            netscape(&[a, b]),
            "# Netscape HTTP Cookie File\n\n\
             #HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t1900000000\tSAPISID\ta b;c=\"d\"\n\
             www.twitch.tv\tFALSE\t/videos\tFALSE\t0\tauth-token\tx\n"
        );
    }

    #[test]
    fn tri_par_site_et_caracteres_interdits() {
        let v = vec![
            c("LOGIN_INFO", "1", ".youtube.com"),
            c("PREF", "2", "www.youtube.com"),
            c("SID", "3", ".google.com"),
            c("tab", "a\tb", ".youtube.com"),
            c("nl", "a\nb", ".youtube.com"),
            c("", "vide", ".youtube.com"),
            c("#x", "commentaire", ".youtube.com"),
            c("pirate", "1", ".notyoutube.com"),
        ];
        let noms = |u: &str| {
            pour_la_page(u, v.clone())
                .into_iter()
                .map(|c| c.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            noms("https://www.youtube.com/watch?v=x"),
            ["LOGIN_INFO", "PREF"]
        );
        assert_eq!(noms("https://youtu.be/x"), ["LOGIN_INFO", "PREF"]);
        assert!(noms("https://www.twitch.tv/videos/1").is_empty());
        let t = vec![c("auth-token", "t", ".twitch.tv")];
        assert_eq!(
            pour_la_page("https://clips.twitch.tv/Abc", t.clone()).len(),
            1
        );
        assert_eq!(pour_la_page("https://u:p@www.twitch.tv:443/x", t).len(), 1);
        // Hôte exact ou sous-domaine seulement : jamais un voisin sous un suffixe public.
        let v = vec![
            c("a", "1", "a.co.uk"),
            c("b", "2", "b.co.uk"),
            c("x", "3", "x.github.io"),
            c("y", "4", ".y.github.io"),
            c("w", "5", "www.a.co.uk"),
            c("t", "6", ".uk"),
            c("i", "7", "io"),
        ];
        let noms = |u: &str| {
            pour_la_page(u, v.clone())
                .into_iter()
                .map(|c| c.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(noms("https://a.co.uk/v"), ["a", "w"]);
        assert_eq!(noms("https://x.github.io/v"), ["x"]);
        assert_eq!(noms("https://cdn.y.github.io/v"), ["y"]);
        assert!(noms("https://z.github.io/v").is_empty());
        let trop = vec![c("a", "b", ".twitch.tv"); MAX + 5];
        assert_eq!(pour_la_page("https://twitch.tv/", trop).len(), MAX);
        // La valeur n'apparaît jamais dans un {:?}.
        assert!(!format!("{:?}", c("SID", "secret123", ".youtube.com")).contains("secret123"));
    }

    #[test]
    fn fichier_prive_efface_apres_usage() {
        let d = std::env::temp_dir().join(format!("fluxdl-cookies-{}", std::process::id()));
        let f = Fichier::creer(&d, &[c("a", "b", ".twitch.tv")]).unwrap();
        let p = f.chemin().to_path_buf();
        assert!(std::fs::read_to_string(&p)
            .unwrap()
            .starts_with("# Netscape HTTP Cookie File\n"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(m(&p), 0o600);
            assert_eq!(m(&d), 0o700);
        }
        drop(f);
        assert!(!p.exists());
        // Restes d'un plantage : effacés au lancement.
        std::fs::write(d.join("vieux.txt"), "x").unwrap();
        purger(&d);
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0);
        let _ = std::fs::remove_dir(&d);
    }
}
