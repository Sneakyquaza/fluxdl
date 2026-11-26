//! Petit serveur HTTP local pour l'extension navigateur : `127.0.0.1` seulement, jeton
//! aléatoire de l'appli exigé (`X-Fluxdl-Token`), `Origin` d'extension (ou `fluxdl://local` pour
//! une 2e instance de l'appli) ou absent, `Host` local (contre le DNS rebinding), débit limité.
//! `Origin` absent accepté : Firefox peut l'omettre pour une extension ; une page web, elle,
//! l'envoie toujours dès qu'elle ajoute l'en-tête du jeton (requête CORS), donc refusée.
//! `GET /ping` → version ; `POST /add` {url, referrer?, filename?, cookies?} → file d'attente.
//! `POST /pair` {nonce} (sans jeton, origine d'extension exigée) : appairage automatique. La 1re
//! requête ouvre « Autoriser ? » dans l'appli (202) ; l'extension la répète avec le même nonce
//! jusqu'à la réponse : jeton (200, une seule fois), refus (403) ou délai passé (410, 60 s).
//! Une seule demande à la fois (409 pour les autres), 3 nouvelles demandes par minute au plus.

use serde::Deserialize;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const PORT: u16 = 47591;
/// Origine envoyée par une 2e instance (lien magnet, .torrent ouvert depuis l'Explorateur).
pub const ORIGINE_LOCALE: &str = "fluxdl://local";
/// Requêtes au plus par fenêtre de 10 s (toutes origines confondues).
const LIMITE: usize = 30;
const MAX_ENTETE: usize = 8 << 10;
const MAX_CORPS: usize = 64 << 10;
/// Durée de vie d'une demande d'appairage (réponse comprise).
pub const DELAI_APPAIRAGE: Duration = Duration::from_secs(60);
/// Nouvelles demandes d'appairage au plus par minute.
const APPAIRAGES_MINUTE: usize = 3;

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Demande {
    pub url: String,
    #[serde(default)]
    pub referrer: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub cookies: Option<String>,
    /// Vrai si la demande vient de l'appli elle-même (fichier .torrent local permis).
    #[serde(skip)]
    pub locale: bool,
}

/// Jeton aléatoire (128 bits en hexadécimal).
pub fn nouveau_jeton() -> String {
    use ring::rand::SecureRandom;
    let mut o = [0u8; 16];
    // raccourci : sans source aléatoire (jamais vu), jeton dérivé de l'heure ; à revoir si ça arrive.
    if ring::rand::SystemRandom::new().fill(&mut o).is_err() {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        o = n.to_le_bytes();
    }
    dlcore::engine::hex(&o)
}

/// Fenêtre glissante des dernières requêtes.
#[derive(Default)]
pub struct Limiteur(VecDeque<Instant>);

impl Limiteur {
    fn accepte(&mut self, t: Instant) -> bool {
        while self
            .0
            .front()
            .is_some_and(|d| t.duration_since(*d) > Duration::from_secs(10))
        {
            self.0.pop_front();
        }
        self.0.push_back(t);
        self.0.len() <= LIMITE
    }
}

/// Demande d'appairage en cours : extension, navigateur deviné, nonce de l'extension,
/// arrivée (sert aussi d'identifiant pour l'UI) et réponse de l'utilisateur.
pub struct Attente {
    pub origine: String,
    pub navigateur: &'static str,
    nonce: String,
    pub depuis: Instant,
    accord: Option<bool>,
}

/// Machine à états de l'appairage, partagée entre le serveur et l'UI.
#[derive(Default)]
pub struct Appairage {
    attente: Option<Attente>,
    essais: VecDeque<Instant>,
}

#[derive(Debug, PartialEq)]
pub enum Etape {
    /// En attente du clic (vrai = nouvelle demande : réveiller l'UI).
    Attente(bool),
    Jeton,
    Refus,
    Occupe,
    Expire,
    Limite,
}

impl Appairage {
    /// Requête `/pair` de `origine` avec `nonce` à l'instant `t`.
    pub fn demander(
        &mut self,
        origine: &str,
        navigateur: &'static str,
        nonce: &str,
        t: Instant,
    ) -> Etape {
        if let Some(a) = &self.attente {
            let meme = a.origine == origine && egal(&a.nonce, nonce);
            if t.duration_since(a.depuis) > DELAI_APPAIRAGE {
                self.attente = None;
                if meme {
                    return Etape::Expire;
                }
            } else if !meme {
                return Etape::Occupe;
            } else {
                return match a.accord {
                    None => Etape::Attente(false),
                    // Réponse donnée une seule fois : la demande disparaît.
                    Some(oui) => {
                        self.attente = None;
                        if oui {
                            Etape::Jeton
                        } else {
                            Etape::Refus
                        }
                    }
                };
            }
        }
        while self
            .essais
            .front()
            .is_some_and(|d| t.duration_since(*d) > Duration::from_secs(60))
        {
            self.essais.pop_front();
        }
        if self.essais.len() >= APPAIRAGES_MINUTE {
            return Etape::Limite;
        }
        self.essais.push_back(t);
        self.attente = Some(Attente {
            origine: origine.into(),
            navigateur,
            nonce: nonce.into(),
            depuis: t,
            accord: None,
        });
        Etape::Attente(true)
    }

    /// Demande à montrer à l'utilisateur : sans réponse et pas expirée.
    pub fn en_cours(&self, t: Instant) -> Option<&Attente> {
        self.attente
            .as_ref()
            .filter(|a| a.accord.is_none() && t.duration_since(a.depuis) <= DELAI_APPAIRAGE)
    }

    /// Réponse de l'utilisateur à la demande arrivée à `depuis` (pas à une autre arrivée
    /// entre-temps).
    pub fn repondre(&mut self, depuis: Instant, oui: bool, t: Instant) {
        if let Some(a) = &mut self.attente {
            if a.depuis == depuis
                && a.accord.is_none()
                && t.duration_since(depuis) <= DELAI_APPAIRAGE
            {
                a.accord = Some(oui);
            }
        }
    }
}

/// Navigateur affiché dans « Autoriser ? » (deviné, à titre indicatif).
fn navigateur(origine: &str, agent: &str) -> &'static str {
    if origine.starts_with("moz-extension://") {
        "Firefox"
    } else if agent.contains("Edg/") {
        "Edge"
    } else if agent.contains("OPR/") {
        "Opera"
    } else if agent.contains("Vivaldi") {
        "Vivaldi"
    } else {
        "Chrome / Brave"
    }
}

/// Réponse : code, corps JSON, origine à renvoyer (CORS), demande acceptée, nouvelle demande
/// d'appairage (réveiller l'UI).
#[derive(Debug, PartialEq)]
pub struct Reponse {
    pub code: u16,
    pub corps: String,
    pub origine: Option<String>,
    pub demande: Option<Demande>,
    pub appairage: bool,
}

fn rep(code: u16, corps: &str) -> Reponse {
    Reponse {
        code,
        corps: corps.into(),
        origine: None,
        demande: None,
        appairage: false,
    }
}

/// Égalité sans fuite de durée (le jeton est un secret).
fn egal(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0, |d, (x, y)| d | (x ^ y)) == 0
}

/// Traite une requête brute (en-têtes + corps). Sans réseau : testable.
pub fn traiter(
    brut: &[u8],
    port: u16,
    jeton: &str,
    lim: &mut Limiteur,
    pair: &mut Appairage,
    t: Instant,
) -> Reponse {
    if !lim.accepte(t) {
        return rep(429, r#"{"error":"rate limited"}"#);
    }
    let Some(fin) = brut.windows(4).position(|w| w == b"\r\n\r\n") else {
        return rep(400, r#"{"error":"bad request"}"#);
    };
    let tete = String::from_utf8_lossy(&brut[..fin]);
    let corps = &brut[fin + 4..];
    let mut lignes = tete.split("\r\n");
    let premiere = lignes.next().unwrap_or("");
    let mut m = premiere.split(' ');
    let (methode, chemin) = (m.next().unwrap_or(""), m.next().unwrap_or(""));
    let entete = |nom: &str| {
        tete.split("\r\n").skip(1).find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case(nom)
                .then(|| v.trim().to_string())
        })
    };
    // Host local exact : une page web qui ferait pointer son domaine sur 127.0.0.1 est refusée.
    let host = entete("Host").unwrap_or_default();
    if host != format!("127.0.0.1:{port}") && host != format!("localhost:{port}") {
        return rep(403, r#"{"error":"bad host"}"#);
    }
    let origine = entete("Origin").unwrap_or_default();
    let extension =
        origine.starts_with("chrome-extension://") || origine.starts_with("moz-extension://");
    if !extension && origine != ORIGINE_LOCALE && !origine.is_empty() {
        return rep(403, r#"{"error":"bad origin"}"#);
    }
    let cors = extension.then(|| origine.clone());
    let avec = |mut r: Reponse| {
        r.origine = cors.clone();
        r
    };
    if methode == "OPTIONS" {
        return avec(rep(204, ""));
    }
    if (methode, chemin) == ("POST", "/pair") {
        // Origine d'extension exigée (une 2e instance ou une requête sans origine n'appaire pas).
        if !extension {
            return rep(403, r#"{"error":"bad origin"}"#);
        }
        if jeton.is_empty() {
            return avec(rep(403, r#"{"error":"server off"}"#));
        }
        #[derive(Deserialize)]
        struct Corps {
            nonce: String,
        }
        let nonce = serde_json::from_slice::<Corps>(corps).map(|c| c.nonce);
        let Some(nonce) = nonce
            .ok()
            .filter(|n| (16..=64).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_hexdigit()))
        else {
            return avec(rep(400, r#"{"error":"bad body"}"#));
        };
        let nav = navigateur(&origine, &entete("User-Agent").unwrap_or_default());
        return match pair.demander(&origine, nav, &nonce, t) {
            Etape::Attente(nouvelle) => {
                let mut r = avec(rep(202, r#"{"state":"waiting"}"#));
                r.appairage = nouvelle;
                r
            }
            Etape::Jeton => avec(rep(200, &serde_json::json!({ "token": jeton }).to_string())),
            Etape::Refus => avec(rep(403, r#"{"error":"denied"}"#)),
            Etape::Occupe => avec(rep(409, r#"{"error":"busy"}"#)),
            Etape::Expire => avec(rep(410, r#"{"error":"expired"}"#)),
            Etape::Limite => avec(rep(429, r#"{"error":"rate limited"}"#)),
        };
    }
    if jeton.is_empty() || !egal(&entete("X-Fluxdl-Token").unwrap_or_default(), jeton) {
        return avec(rep(401, r#"{"error":"bad token"}"#));
    }
    match (methode, chemin) {
        ("GET", "/ping") => avec(rep(
            200,
            &format!(
                r#"{{"app":"{}","version":"{}"}}"#,
                dlcore::APP_NAME,
                env!("CARGO_PKG_VERSION")
            ),
        )),
        ("POST", "/add") => {
            let Ok(mut d) = serde_json::from_slice::<Demande>(corps) else {
                return avec(rep(400, r#"{"error":"bad body"}"#));
            };
            d.locale = origine == ORIGINE_LOCALE;
            let u = d.url.trim();
            let permis = u.starts_with("http://")
                || u.starts_with("https://")
                || u.starts_with("magnet:")
                || (d.locale && u.to_ascii_lowercase().ends_with(".torrent"));
            if !permis || u.len() > 16 << 10 {
                return avec(rep(400, r#"{"error":"bad url"}"#));
            }
            let mut r = avec(rep(200, r#"{"ok":true}"#));
            r.demande = Some(d);
            r
        }
        _ => avec(rep(404, r#"{"error":"not found"}"#)),
    }
}

/// Lit une requête : en-têtes puis `Content-Length` octets (tailles bornées).
fn lire(s: &mut TcpStream) -> Option<Vec<u8>> {
    s.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    let mut d = Vec::new();
    let mut tampon = [0u8; 4096];
    loop {
        let n = s.read(&mut tampon).ok()?;
        if n == 0 {
            return None;
        }
        d.extend_from_slice(&tampon[..n]);
        if let Some(fin) = d.windows(4).position(|w| w == b"\r\n\r\n") {
            let tete = String::from_utf8_lossy(&d[..fin]).to_ascii_lowercase();
            let longueur = tete
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if longueur > MAX_CORPS {
                return None;
            }
            while d.len() < fin + 4 + longueur {
                let n = s.read(&mut tampon).ok()?;
                if n == 0 {
                    return None;
                }
                d.extend_from_slice(&tampon[..n]);
            }
            return Some(d);
        }
        if d.len() > MAX_ENTETE {
            return None;
        }
    }
}

fn repondre(s: &mut TcpStream, r: &Reponse) {
    let texte = match r.code {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        409 => "Conflict",
        410 => "Gone",
        429 => "Too Many Requests",
        _ => "Not Found",
    };
    let mut h = format!(
        "HTTP/1.1 {} {texte}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        r.code,
        r.corps.len()
    );
    if let Some(o) = &r.origine {
        h += &format!(
            "Access-Control-Allow-Origin: {o}\r\nAccess-Control-Allow-Headers: Content-Type, X-Fluxdl-Token\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nVary: Origin\r\n"
        );
    }
    h += "\r\n";
    let _ = s.write_all(h.as_bytes());
    let _ = s.write_all(r.corps.as_bytes());
}

/// Écoute `127.0.0.1:port` dans un fil ; chaque demande acceptée part dans `tx` et réveille
/// l'interface. Renvoie le port réel (utile avec 0 dans les tests).
pub fn lancer(
    port: u16,
    jeton: Arc<Mutex<String>>,
    pair: Arc<Mutex<Appairage>>,
    tx: Sender<Demande>,
    reveil: impl Fn() + Send + 'static,
) -> std::io::Result<u16> {
    let ecoute = TcpListener::bind(("127.0.0.1", port))?;
    let port = ecoute.local_addr()?.port();
    std::thread::spawn(move || {
        let mut lim = Limiteur::default();
        for s in ecoute.incoming() {
            let Ok(mut s) = s else { continue };
            // raccourci : une connexion à la fois (requêtes minuscules, délai de 3 s max).
            let Some(brut) = lire(&mut s) else { continue };
            let j = jeton.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let r = {
                let mut p = pair.lock().unwrap_or_else(|e| e.into_inner());
                traiter(&brut, port, &j, &mut lim, &mut p, Instant::now())
            };
            repondre(&mut s, &r);
            if r.appairage {
                reveil();
            }
            if let Some(d) = r.demande {
                if tx.send(d).is_err() {
                    return;
                }
                reveil();
            }
        }
    });
    Ok(port)
}

/// 2e instance : confie `url` (lien magnet, chemin de .torrent) à l'appli déjà lancée.
pub fn confier(port: u16, jeton: &str, url: &str) -> bool {
    let corps = serde_json::json!({ "url": url }).to_string();
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(2))
        .build()
        .post(&format!("http://127.0.0.1:{port}/add"))
        .set("Origin", ORIGINE_LOCALE)
        .set("X-Fluxdl-Token", jeton)
        .set("Content-Type", "application/json")
        .send_string(&corps)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const J: &str = "0123456789abcdef0123456789abcdef";

    fn req(methode: &str, chemin: &str, entetes: &[(&str, &str)], corps: &str) -> Vec<u8> {
        let mut s = format!("{methode} {chemin} HTTP/1.1\r\n");
        for (k, v) in entetes {
            s += &format!("{k}: {v}\r\n");
        }
        s += &format!("Content-Length: {}\r\n\r\n{corps}", corps.len());
        s.into_bytes()
    }

    fn ext(jeton: &str) -> Vec<(&'static str, String)> {
        vec![
            ("Host", "127.0.0.1:47591".into()),
            ("Origin", "chrome-extension://abcdefghijklmnop".into()),
            ("X-Fluxdl-Token", jeton.into()),
        ]
    }

    fn envoyer(e: &[(&str, String)], methode: &str, chemin: &str, corps: &str) -> Reponse {
        let e: Vec<(&str, &str)> = e.iter().map(|(k, v)| (*k, v.as_str())).collect();
        traiter(
            &req(methode, chemin, &e, corps),
            PORT,
            J,
            &mut Limiteur::default(),
            &mut Appairage::default(),
            Instant::now(),
        )
    }

    #[test]
    fn jeton_origine_et_hote() {
        let ok = envoyer(&ext(J), "GET", "/ping", "");
        assert_eq!(ok.code, 200);
        assert!(ok.corps.contains("fluxdl"));
        assert_eq!(
            ok.origine.as_deref(),
            Some("chrome-extension://abcdefghijklmnop")
        );
        let add = envoyer(
            &ext(J),
            "POST",
            "/add",
            r#"{"url":"https://a.b/f.zip","referrer":"https://a.b/"}"#,
        );
        assert_eq!(add.code, 200);
        let d = add.demande.unwrap();
        assert_eq!((d.url.as_str(), d.locale), ("https://a.b/f.zip", false));
        assert_eq!(d.referrer.as_deref(), Some("https://a.b/"));
        // Jeton faux, absent ou vide côté appli.
        assert_eq!(envoyer(&ext("mauvais"), "GET", "/ping", "").code, 401);
        let sans: Vec<_> = ext(J)
            .into_iter()
            .filter(|e| e.0 != "X-Fluxdl-Token")
            .collect();
        assert_eq!(envoyer(&sans, "GET", "/ping", "").code, 401);
        // Origine d'une page web : refusée même avec le bon jeton.
        for o in ["https://pirate.example", "http://127.0.0.1:8000", "null"] {
            let mut e = ext(J);
            e[1].1 = o.into();
            assert_eq!(
                envoyer(&e, "POST", "/add", r#"{"url":"https://x/y"}"#).code,
                403,
                "{o}"
            );
        }
        // Sans origine (Firefox) : le jeton seul décide.
        let sans: Vec<_> = ext(J).into_iter().filter(|e| e.0 != "Origin").collect();
        assert_eq!(envoyer(&sans, "GET", "/ping", "").code, 200);
        let sans_tout: Vec<_> = sans
            .into_iter()
            .filter(|e| e.0 != "X-Fluxdl-Token")
            .collect();
        assert_eq!(envoyer(&sans_tout, "GET", "/ping", "").code, 401);
        // Host d'un domaine qui pointe vers 127.0.0.1 (DNS rebinding) : refusé.
        let mut e = ext(J);
        e[0].1 = "pirate.example:47591".into();
        assert_eq!(envoyer(&e, "GET", "/ping", "").code, 403);
        // Firefox.
        let mut e = ext(J);
        e[1].1 = "moz-extension://3f2a-uuid".into();
        assert_eq!(envoyer(&e, "GET", "/ping", "").code, 200);
    }

    #[test]
    fn corps_et_liens() {
        assert_eq!(envoyer(&ext(J), "POST", "/add", "pas du json").code, 400);
        assert_eq!(
            envoyer(&ext(J), "POST", "/add", r#"{"url":"file:///etc/passwd"}"#).code,
            400
        );
        // Un .torrent local : seulement depuis l'appli elle-même.
        let t = r#"{"url":"C:\\Users\\a\\b.torrent"}"#;
        assert_eq!(envoyer(&ext(J), "POST", "/add", t).code, 400);
        let mut e = ext(J);
        e[1].1 = ORIGINE_LOCALE.into();
        let r = envoyer(&e, "POST", "/add", t);
        assert_eq!(r.code, 200);
        assert!(r.demande.unwrap().locale);
        assert!(
            envoyer(&e, "POST", "/add", r#"{"url":"magnet:?xt=urn:btih:abc"}"#)
                .demande
                .is_some()
        );
        assert_eq!(envoyer(&ext(J), "GET", "/ailleurs", "").code, 404);
        assert_eq!(envoyer(&ext(J), "OPTIONS", "/add", "").code, 204);
    }

    #[test]
    fn debit_limite() {
        let mut lim = Limiteur::default();
        let t = Instant::now();
        let e: Vec<(&str, String)> = ext(J);
        let e: Vec<(&str, &str)> = e.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let r = req("GET", "/ping", &e, "");
        for _ in 0..LIMITE {
            assert_eq!(
                traiter(&r, PORT, J, &mut lim, &mut Appairage::default(), t).code,
                200
            );
        }
        assert_eq!(
            traiter(&r, PORT, J, &mut lim, &mut Appairage::default(), t).code,
            429
        );
        // 10 s plus tard, la fenêtre est vide.
        assert_eq!(
            traiter(
                &r,
                PORT,
                J,
                &mut lim,
                &mut Appairage::default(),
                t + Duration::from_secs(11)
            )
            .code,
            200
        );
    }

    const N: &str = "00112233445566778899aabbccddeeff";

    /// Requête `/pair` sans jeton depuis `origine` (vide = sans en-tête Origin).
    fn pair(origine: &str, nonce: &str, p: &mut Appairage, t: Instant) -> Reponse {
        let mut e = vec![("Host", "127.0.0.1:47591")];
        if !origine.is_empty() {
            e.push(("Origin", origine));
        }
        let corps = format!(r#"{{"nonce":"{nonce}"}}"#);
        traiter(
            &req("POST", "/pair", &e, &corps),
            PORT,
            J,
            &mut Limiteur::default(),
            p,
            t,
        )
    }

    #[test]
    fn appairage_accorde_une_fois() {
        let (mut p, t) = (Appairage::default(), Instant::now());
        let o = "chrome-extension://abcdefghijklmnop";
        let r = pair(o, N, &mut p, t);
        assert_eq!((r.code, r.appairage), (202, true));
        assert!(!r.corps.contains(J));
        let a = p.en_cours(t).unwrap();
        assert_eq!((a.origine.as_str(), a.navigateur), (o, "Chrome / Brave"));
        let depuis = a.depuis;
        // Sans clic : jamais de jeton, l'UI n'est réveillée qu'une fois.
        let r = pair(o, N, &mut p, t + Duration::from_secs(5));
        assert_eq!((r.code, r.appairage), (202, false));
        // Une autre extension (ou un autre nonce) pendant l'attente : occupé, sans jeton.
        let r = pair("chrome-extension://zzzz", N, &mut p, t);
        assert_eq!(r.code, 409);
        assert_eq!(pair(o, &N.replace('0', "1"), &mut p, t).code, 409);
        // Clic « Autoriser » pour une autre demande (identifiant différent) : ignoré.
        p.repondre(depuis + Duration::from_millis(1), true, t);
        assert_eq!(pair(o, N, &mut p, t).code, 202);
        p.repondre(depuis, true, t + Duration::from_secs(10));
        assert!(p.en_cours(t + Duration::from_secs(10)).is_none());
        // L'intrus ne l'obtient pas ; le demandeur, une seule fois.
        assert_eq!(pair("chrome-extension://zzzz", N, &mut p, t).code, 409);
        let r = pair(o, N, &mut p, t + Duration::from_secs(11));
        assert_eq!(r.code, 200);
        assert_eq!(r.corps, format!(r#"{{"token":"{J}"}}"#));
        assert_eq!(r.origine.as_deref(), Some(o));
        // Rejeu de la même requête : nouvelle demande, pas de jeton.
        let r = pair(o, N, &mut p, t + Duration::from_secs(12));
        assert_eq!((r.code, r.appairage), (202, true));
        assert!(!r.corps.contains(J));
    }

    #[test]
    fn appairage_refus_expiration_limite() {
        let (mut p, t) = (Appairage::default(), Instant::now());
        let o = "moz-extension://3f2a-uuid";
        assert_eq!(pair(o, N, &mut p, t).code, 202);
        assert_eq!(p.en_cours(t).unwrap().navigateur, "Firefox");
        p.repondre(t, false, t);
        assert_eq!(pair(o, N, &mut p, t).code, 403);
        // Expiration : plus montrée après 60 s, puis 410 au demandeur ; un clic tardif est ignoré.
        let t2 = t + Duration::from_secs(1);
        assert_eq!(pair(o, N, &mut p, t2).code, 202);
        let tard = t2 + DELAI_APPAIRAGE + Duration::from_secs(1);
        assert!(p.en_cours(tard).is_none());
        p.repondre(t2, true, tard);
        assert_eq!(pair(o, N, &mut p, tard).code, 410);
        // 3 nouvelles demandes par minute au plus.
        let mut p = Appairage::default();
        for _ in 0..APPAIRAGES_MINUTE {
            assert_eq!(pair(o, N, &mut p, t).code, 202);
            p.repondre(t, false, t);
            assert_eq!(pair(o, N, &mut p, t).code, 403);
        }
        assert_eq!(pair(o, N, &mut p, t).code, 429);
        assert!(p.en_cours(t).is_none());
        assert_eq!(pair(o, N, &mut p, t + Duration::from_secs(30)).code, 429);
        assert_eq!(pair(o, N, &mut p, t + Duration::from_secs(61)).code, 202);
    }

    #[test]
    fn appairage_refuse_hors_extension() {
        let (mut p, t) = (Appairage::default(), Instant::now());
        // Page web, origine absente, 2e instance de l'appli : refusés, aucune demande ouverte.
        for o in ["https://pirate.example", "null", "", ORIGINE_LOCALE] {
            let r = pair(o, N, &mut p, t);
            assert_eq!(r.code, 403, "{o}");
            assert!(!r.appairage && !r.corps.contains(J));
        }
        assert!(p.en_cours(t).is_none());
        // Host d'un domaine pirate (DNS rebinding) : refusé.
        let e = [
            ("Host", "pirate.example:47591"),
            ("Origin", "chrome-extension://abcdefghijklmnop"),
        ];
        let r = traiter(
            &req("POST", "/pair", &e, &format!(r#"{{"nonce":"{N}"}}"#)),
            PORT,
            J,
            &mut Limiteur::default(),
            &mut p,
            t,
        );
        assert_eq!(r.code, 403);
        // Nonce absent, trop court ou non hexadécimal.
        let o = "chrome-extension://abcdefghijklmnop";
        for n in ["", "abc", "zz112233445566778899aabbccddeeff"] {
            assert_eq!(pair(o, n, &mut p, t).code, 400, "{n}");
        }
        // Serveur coupé (jeton vide) : rien.
        let e = [("Host", "127.0.0.1:47591"), ("Origin", o)];
        let r = traiter(
            &req("POST", "/pair", &e, &format!(r#"{{"nonce":"{N}"}}"#)),
            PORT,
            "",
            &mut Limiteur::default(),
            &mut p,
            t,
        );
        assert_eq!(r.code, 403);
        assert!(p.en_cours(t).is_none());
        assert_eq!(
            navigateur(o, "Mozilla/5.0 … Chrome/130 Safari/537.36 Edg/130"),
            "Edge"
        );
    }

    #[test]
    fn vrai_serveur_local() {
        let (tx, rx) = std::sync::mpsc::channel();
        let jeton = Arc::new(Mutex::new(J.to_string()));
        let port = lancer(0, jeton, Arc::default(), tx, || {}).unwrap();
        assert!(confier(port, J, "magnet:?xt=urn:btih:abc"));
        let d = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(d.url, "magnet:?xt=urn:btih:abc");
        assert!(!confier(port, "faux", "magnet:?xt=urn:btih:abc"));
    }
}
