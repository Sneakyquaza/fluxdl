//! Petit serveur HTTP local pour l'extension navigateur : `127.0.0.1` seulement, jeton
//! aléatoire de l'appli exigé (`X-Fluxdl-Token`), `Origin` d'extension (ou `fluxdl://local` pour
//! une 2e instance de l'appli) ou absent, `Host` local (contre le DNS rebinding), débit limité.
//! `Origin` absent accepté : Firefox peut l'omettre pour une extension ; une page web, elle,
//! l'envoie toujours dès qu'elle ajoute l'en-tête du jeton (requête CORS), donc refusée.
//! `GET /ping` → version ; `POST /add` {url, referrer?, filename?, cookies?} → file d'attente,
//! réponse donnée après la décision de l'appli : 200 `added` / `already_active` (le navigateur
//! peut abandonner sa copie), 409 `already_done` / `refused`, 503 `timeout` (rien d'ajouté).
//! `POST /focus` (2e instance seulement) : montre la fenêtre.
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
/// Requêtes acceptées (Host, Origin et jeton bons) au plus par fenêtre de 10 s.
const LIMITE: usize = 30;
/// Connexions traitées en même temps au plus ; délai total pour lire une requête.
const MAX_CONNEXIONS: usize = 8;
const DELAI_LECTURE: Duration = Duration::from_secs(2);
/// Attente de la décision de l'appli pour un `/add` (l'extension attend un peu plus).
pub const DELAI_DECISION: Duration = Duration::from_secs(5);
const MAX_ENTETE: usize = 8 << 10;
const MAX_CORPS: usize = 64 << 10;
/// Durée de vie d'une demande d'appairage (réponse comprise).
pub const DELAI_APPAIRAGE: Duration = Duration::from_secs(60);
/// Nouvelles demandes d'appairage au plus par minute.
const APPAIRAGES_MINUTE: usize = 3;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Demande {
    pub url: String,
    #[serde(default)]
    pub referrer: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub cookies: Option<String>,
    /// Capture automatique : sonder le lien avant d'accepter (le navigateur abandonne sa copie
    /// sur « added ») ; page web, erreur ou délai → refus, le navigateur garde son téléchargement.
    #[serde(default)]
    pub probe: bool,
    /// Vrai si la demande vient de l'appli elle-même (fichier .torrent local permis).
    #[serde(skip)]
    pub locale: bool,
    /// Réponse attendue par le serveur (absente : rien à répondre).
    #[serde(skip)]
    pub decision: Option<Arc<Decision>>,
}

impl PartialEq for Demande {
    fn eq(&self, o: &Self) -> bool {
        (
            &self.url,
            &self.referrer,
            &self.filename,
            &self.cookies,
            self.locale,
        ) == (&o.url, &o.referrer, &o.filename, &o.cookies, o.locale)
    }
}

/// Sort d'un lien confié à l'appli.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Statut {
    Ajoute,
    DejaActif,
    DejaFini,
    Refuse,
}

impl Statut {
    fn texte(self) -> &'static str {
        match self {
            Statut::Ajoute => "added",
            Statut::DejaActif => "already_active",
            Statut::DejaFini => "already_done",
            Statut::Refuse => "refused",
        }
    }
}

/// Décision de l'UI pour un `/add`, attendue par le fil du serveur. Une fois le délai passé la
/// demande est abandonnée : l'UI ne l'ajoute plus (l'extension a gardé la copie du navigateur).
#[derive(Debug, Default)]
pub struct Decision(Mutex<Option<Result<Statut, ()>>>);

impl Decision {
    /// Décide avec `f` (exécutée sous le verrou) si la demande attend encore.
    pub fn decider(&self, f: impl FnOnce() -> Statut) {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_none() {
            *g = Some(Ok(f()));
        }
    }

    /// Décision de l'UI, sinon abandon au bout de `delai` (`None`).
    pub fn attendre(&self, delai: Duration) -> Option<Statut> {
        let fin = Instant::now() + delai;
        loop {
            {
                let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(r) = *g {
                    return r.ok();
                }
                if Instant::now() >= fin {
                    *g = Some(Err(()));
                    return None;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
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
/// `jeton` : celui de l'extension (vide = serveur coupé) ; `local` : celui d'une 2e instance
/// (toujours actif, l'appli s'écoute même serveur coupé pour recevoir les liens ouverts).
pub fn traiter(
    brut: &[u8],
    port: u16,
    (jeton, local): (&str, &str),
    lim: &mut Limiteur,
    pair: &mut Appairage,
    t: Instant,
) -> Reponse {
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
    let jeton = if origine == ORIGINE_LOCALE {
        local
    } else {
        jeton
    };
    if jeton.is_empty() || !egal(&entete("X-Fluxdl-Token").unwrap_or_default(), jeton) {
        return avec(rep(401, r#"{"error":"bad token"}"#));
    }
    // Seules les requêtes légitimes comptent : une page web ne peut pas épuiser la limite.
    if !lim.accepte(t) {
        return avec(rep(429, r#"{"error":"rate limited"}"#));
    }
    match (methode, chemin) {
        ("POST", "/focus") if origine == ORIGINE_LOCALE => {
            let mut r = rep(200, r#"{"ok":true}"#);
            r.demande = Some(Demande {
                locale: true,
                ..Default::default()
            });
            r
        }
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
    // Délai total (pas par lecture) : un octet toutes les 2,5 s ne retient pas la connexion.
    let fin = Instant::now() + DELAI_LECTURE;
    let lire_un = |s: &mut TcpStream, b: &mut [u8]| {
        let reste = fin.checked_duration_since(Instant::now())?;
        s.set_read_timeout(Some(reste.max(Duration::from_millis(1))))
            .ok()?;
        s.read(b).ok()
    };
    let mut d = Vec::new();
    let mut tampon = [0u8; 4096];
    loop {
        let n = lire_un(s, &mut tampon)?;
        if n == 0 {
            return None;
        }
        d.extend_from_slice(&tampon[..n]);
        if let Some(f) = d.windows(4).position(|w| w == b"\r\n\r\n") {
            let tete = String::from_utf8_lossy(&d[..f]).to_ascii_lowercase();
            let longueur = tete
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if longueur > MAX_CORPS {
                return None;
            }
            while d.len() < f + 4 + longueur {
                let n = lire_un(s, &mut tampon)?;
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
        503 => "Service Unavailable",
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

/// Écoute `127.0.0.1:port` dans un fil (un fil par connexion, `MAX_CONNEXIONS` au plus) ;
/// chaque demande part dans `tx` et réveille l'interface, qui décide (`Decision`) avant la
/// réponse. `local` : jeton d'une 2e instance. Renvoie le port réel (utile avec 0 dans les
/// tests). Port pris (l'ancienne version finit de se fermer après une mise à jour) : nouvel
/// essai pendant 5 s.
pub fn lancer(
    port: u16,
    jeton: Arc<Mutex<String>>,
    local: String,
    pair: Arc<Mutex<Appairage>>,
    tx: Sender<Demande>,
    reveil: impl Fn() + Send + Sync + 'static,
) -> std::io::Result<u16> {
    let fin = Instant::now() + Duration::from_secs(5);
    let ecoute = loop {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(e) => break e,
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && Instant::now() < fin => {
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(e) => return Err(e),
        }
    };
    let port = ecoute.local_addr()?.port();
    let lim = Arc::new(Mutex::new(Limiteur::default()));
    let reveil = Arc::new(reveil);
    let actives = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    std::thread::spawn(move || {
        use std::sync::atomic::Ordering::SeqCst;
        for s in ecoute.incoming() {
            let Ok(mut s) = s else { continue };
            if actives.fetch_add(1, SeqCst) >= MAX_CONNEXIONS {
                actives.fetch_sub(1, SeqCst);
                continue;
            }
            let (jeton, local, pair, lim, tx, reveil, actives) = (
                jeton.clone(),
                local.clone(),
                pair.clone(),
                lim.clone(),
                tx.clone(),
                reveil.clone(),
                actives.clone(),
            );
            std::thread::spawn(move || {
                servir(&mut s, port, &jeton, &local, &pair, &lim, &tx, &*reveil);
                actives.fetch_sub(1, SeqCst);
            });
        }
    });
    Ok(port)
}

/// Le lien mène-t-il à un fichier (sonde avec le référent et les cookies de l'extension, 5 s
/// au plus) ? Une page de connexion, un POST rejoué en GET ou une erreur : non.
fn sonde_fichier(d: &Demande) -> bool {
    use dlcore::engine;
    let mut h = Vec::new();
    if let Some(r) = &d.referrer {
        h.push(("Referer".to_string(), r.clone()));
    }
    if let Some(c) = &d.cookies {
        h.push(("Cookie".to_string(), c.clone()));
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let url = d.url.trim().to_string();
    std::thread::spawn(move || {
        let ok =
            engine::probe(&engine::agent(), &url, &h).is_ok_and(|p| !engine::est_page(&p, &url));
        let _ = tx.send(ok);
    });
    rx.recv_timeout(Duration::from_secs(5)).unwrap_or(false)
}

/// Une connexion : lecture, traitement, décision de l'UI pour un `/add`, réponse.
#[allow(clippy::too_many_arguments)]
fn servir(
    s: &mut TcpStream,
    port: u16,
    jeton: &Mutex<String>,
    local: &str,
    pair: &Mutex<Appairage>,
    lim: &Mutex<Limiteur>,
    tx: &Sender<Demande>,
    reveil: &dyn Fn(),
) {
    let Some(brut) = lire(s) else { return };
    let j = jeton.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut r = {
        let mut p = pair.lock().unwrap_or_else(|e| e.into_inner());
        let mut l = lim.lock().unwrap_or_else(|e| e.into_inner());
        traiter(&brut, port, (&j, local), &mut l, &mut p, Instant::now())
    };
    if let Some(mut d) = r.demande.take() {
        if d.probe && !sonde_fichier(&d) {
            r.code = 409;
            r.corps = r#"{"ok":false,"status":"refused"}"#.into();
            repondre(s, &r);
            return;
        }
        let attendre = !d.url.is_empty();
        let dec = Arc::new(Decision::default());
        if attendre {
            d.decision = Some(dec.clone());
        }
        if tx.send(d).is_err() {
            return;
        }
        reveil();
        if attendre {
            let (code, statut) = match dec.attendre(DELAI_DECISION) {
                Some(st @ (Statut::Ajoute | Statut::DejaActif)) => (200, st.texte()),
                Some(st) => (409, st.texte()),
                None => (503, "timeout"),
            };
            r.code = code;
            r.corps = serde_json::json!({ "ok": code == 200, "status": statut }).to_string();
        }
    }
    repondre(s, &r);
    if r.appairage {
        reveil();
    }
}

/// 2e instance : confie `url` (lien magnet, chemin de .torrent) à l'appli déjà lancée.
pub fn confier(port: u16, jeton: &str, url: &str) -> bool {
    let corps = serde_json::json!({ "url": url }).to_string();
    poster(port, jeton, "/add", &corps)
}

/// 2e instance lancée sans lien : la 1re montre sa fenêtre.
pub fn montrer(port: u16, jeton: &str) -> bool {
    poster(port, jeton, "/focus", "{}")
}

fn poster(port: u16, jeton: &str, chemin: &str, corps: &str) -> bool {
    let r = ureq::AgentBuilder::new()
        .timeout(DELAI_DECISION + Duration::from_secs(2))
        .build()
        .post(&format!("http://127.0.0.1:{port}{chemin}"))
        .set("Origin", ORIGINE_LOCALE)
        .set("X-Fluxdl-Token", jeton)
        .set("Content-Type", "application/json")
        .send_string(corps);
    // 409 (déjà là, refusé) : l'appli a bien reçu le lien.
    matches!(r, Ok(_) | Err(ureq::Error::Status(409, _)))
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
            (J, J),
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
                traiter(&r, PORT, (J, J), &mut lim, &mut Appairage::default(), t).code,
                200
            );
        }
        assert_eq!(
            traiter(&r, PORT, (J, J), &mut lim, &mut Appairage::default(), t).code,
            429
        );
        // 10 s plus tard, la fenêtre est vide.
        assert_eq!(
            traiter(
                &r,
                PORT,
                (J, J),
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
            (J, J),
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
            (J, J),
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
            ("", J),
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
        let (tx, rx) = std::sync::mpsc::channel::<Demande>();
        let jeton = Arc::new(Mutex::new(J.to_string()));
        let port = lancer(0, jeton.clone(), J.into(), Arc::default(), tx, || {}).unwrap();
        // L'« UI » : déjà active, puis refus ; la réponse attend la décision.
        let ui = std::thread::spawn(move || {
            let mut vus = Vec::new();
            for st in [Statut::DejaActif, Statut::Refuse] {
                let d = rx.recv_timeout(Duration::from_secs(3)).unwrap();
                d.decision.as_ref().unwrap().decider(|| st);
                vus.push(d.url);
            }
            // /focus : sans décision à donner.
            let d = rx.recv_timeout(Duration::from_secs(3)).unwrap();
            assert!(d.url.is_empty() && d.decision.is_none() && d.locale);
            vus
        });
        assert!(confier(port, J, "magnet:?xt=urn:btih:abc"));
        assert!(confier(port, J, "magnet:?xt=urn:btih:abc"));
        assert!(montrer(port, J));
        assert_eq!(ui.join().unwrap(), ["magnet:?xt=urn:btih:abc"; 2]);
        assert!(!confier(port, "faux", "magnet:?xt=urn:btih:abc"));
        // Serveur coupé pour l'extension : la 2e instance passe toujours (jeton local).
        *jeton.lock().unwrap() = String::new();
        let e = ext(J);
        let r = ureq::get(&format!("http://127.0.0.1:{port}/ping"))
            .set("Origin", &e[1].1)
            .set("X-Fluxdl-Token", J)
            .call();
        assert!(matches!(r, Err(ureq::Error::Status(401, _))));
    }

    #[test]
    fn decision_abandonnee_apres_le_delai() {
        let d = Decision::default();
        assert_eq!(d.attendre(Duration::from_millis(30)), None);
        // Trop tard : l'UI ne décide plus (rien ne doit être ajouté).
        let mut appele = false;
        d.decider(|| {
            appele = true;
            Statut::Ajoute
        });
        assert!(!appele);
        let d = Decision::default();
        d.decider(|| Statut::DejaFini);
        assert_eq!(d.attendre(Duration::ZERO), Some(Statut::DejaFini));
    }

    #[test]
    fn connexion_lente_ne_bloque_pas_les_autres() {
        let (tx, _rx) = std::sync::mpsc::channel::<Demande>();
        let port = lancer(
            0,
            Arc::new(Mutex::new(J.into())),
            J.into(),
            Arc::default(),
            tx,
            || {},
        )
        .unwrap();
        // Une connexion qui envoie un octet de temps en temps, puis un ping normal.
        let mut lente = TcpStream::connect(("127.0.0.1", port)).unwrap();
        lente.write_all(b"G").unwrap();
        let t = Instant::now();
        let r = ureq::get(&format!("http://127.0.0.1:{port}/ping"))
            .set("Origin", "chrome-extension://abcdefghijklmnop")
            .set("X-Fluxdl-Token", J)
            .call();
        assert!(r.is_ok() && t.elapsed() < Duration::from_secs(1));
        // Pages web refusées : elles n'entament pas la limite de l'extension.
        let web = req(
            "GET",
            "/ping",
            &[("Host", "127.0.0.1:47591"), ("Origin", "https://x.example")],
            "",
        );
        let mut lim = Limiteur::default();
        for _ in 0..LIMITE * 2 {
            let r = traiter(&web, PORT, (J, J), &mut lim, &mut Appairage::default(), t);
            assert_eq!(r.code, 403);
        }
        assert_eq!(envoyer(&ext(J), "GET", "/ping", "").code, 200);
        let e: Vec<(&str, String)> = ext(J);
        let e: Vec<(&str, &str)> = e.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let r = traiter(
            &req("GET", "/ping", &e, ""),
            PORT,
            (J, J),
            &mut lim,
            &mut Appairage::default(),
            t,
        );
        assert_eq!(r.code, 200);
    }

    /// Petit serveur HTTP : chaque requête reçoit `ctype` (206, 4 octets).
    fn site(ctype: &'static str) -> u16 {
        let e = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = e.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for s in e.incoming() {
                let Ok(mut s) = s else { continue };
                let mut b = [0u8; 4096];
                let _ = s.read(&mut b);
                let _ = s.write_all(
                    format!("HTTP/1.1 206 Partial Content\r\nContent-Type: {ctype}\r\nContent-Range: bytes 0-0/4\r\nContent-Length: 1\r\nConnection: close\r\n\r\nx").as_bytes(),
                );
            }
        });
        port
    }

    /// Audit M8 : capture automatique d'un lien qui donne une page (connexion exigée, POST) →
    /// refus avant toute décision de l'UI : le navigateur garde sa copie.
    #[test]
    fn capture_sondee_avant_accord() {
        let page = site("text/html; charset=utf-8");
        let fichier = site("application/zip");
        let (tx, rx) = std::sync::mpsc::channel::<Demande>();
        let port = lancer(
            0,
            Arc::new(Mutex::new(J.into())),
            J.into(),
            Arc::default(),
            tx,
            || {},
        )
        .unwrap();
        // Code HTTP et corps de la réponse.
        let ajout = |u: String| match ureq::post(&format!("http://127.0.0.1:{port}/add"))
            .set("Origin", "chrome-extension://abcdefghijklmnop")
            .set("X-Fluxdl-Token", J)
            .send_string(&serde_json::json!({ "url": u, "probe": true }).to_string())
        {
            Ok(r) => (200, r.into_string().unwrap()),
            Err(ureq::Error::Status(c, _)) => (c, String::new()),
            Err(_) => (0, String::new()),
        };
        let r = ajout(format!("http://127.0.0.1:{page}/telecharger.zip"));
        assert_eq!(r.0, 409);
        assert!(rx.try_recv().is_err(), "rien n'arrive à l'UI");
        let ui = std::thread::spawn(move || {
            let d = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            d.decision.as_ref().unwrap().decider(|| Statut::Ajoute);
        });
        let r = ajout(format!("http://127.0.0.1:{fichier}/a.zip"));
        assert!(r.0 == 200 && r.1.contains("added"));
        ui.join().unwrap();
    }
}
