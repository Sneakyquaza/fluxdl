//! Installation guidée de l'extension navigateur, embarquée dans l'exe (build.rs) :
//! Chrome/Edge/Brave… = dossier décompressé (`Charger l'extension non empaquetée`), remis à
//! jour à chaque version de l'appli ; Firefox = zip chargé comme module temporaire.

use std::io;
use std::path::{Path, PathBuf};

include!(concat!(env!("OUT_DIR"), "/extension.rs"));

/// Navigateur installé : nom affiché, exécutable, page des extensions.
#[derive(Clone, Debug, PartialEq)]
pub struct Navigateur {
    pub nom: &'static str,
    pub exe: PathBuf,
    pub page: &'static str,
    pub firefox: bool,
}

/// (nom, page, chemins Windows sous Program Files / LOCALAPPDATA, exe des App Paths,
/// commandes Linux). Ordre = ordre d'affichage.
type Modele = (
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static str,
    &'static [&'static str],
);
const MODELES: [Modele; 7] = [
    (
        "Chrome",
        "chrome://extensions",
        &[r"Google\Chrome\Application\chrome.exe"],
        "chrome.exe",
        &["google-chrome", "google-chrome-stable"],
    ),
    (
        "Edge",
        "edge://extensions",
        &[r"Microsoft\Edge\Application\msedge.exe"],
        "msedge.exe",
        &["microsoft-edge", "microsoft-edge-stable"],
    ),
    (
        "Brave",
        "brave://extensions",
        &[r"BraveSoftware\Brave-Browser\Application\brave.exe"],
        "brave.exe",
        &["brave-browser", "brave"],
    ),
    (
        "Firefox",
        "about:debugging#/runtime/this-firefox",
        &[r"Mozilla Firefox\firefox.exe"],
        "firefox.exe",
        &["firefox", "firefox-esr"],
    ),
    (
        "Opera",
        "opera://extensions",
        &[r"Programs\Opera\opera.exe"],
        "opera.exe",
        &["opera"],
    ),
    (
        "Vivaldi",
        "vivaldi://extensions",
        &[r"Vivaldi\Application\vivaldi.exe"],
        "vivaldi.exe",
        &["vivaldi", "vivaldi-stable"],
    ),
    (
        "Chromium",
        "chrome://extensions",
        &[r"Chromium\Application\chrome.exe"],
        "",
        &["chromium", "chromium-browser"],
    ),
];

/// Valeurs par défaut des clés `App Paths` (sortie de `reg query … /s`) : (exe en minuscules,
/// chemin). Le nom « (Default) » est traduit selon la langue de Windows : on prend la valeur
/// entre parenthèses.
pub fn app_paths(sortie: &str) -> Vec<(String, String)> {
    let mut cle = String::new();
    let mut v = Vec::new();
    for l in sortie.lines() {
        if l.starts_with("HKEY_") {
            cle = l
                .rsplit('\\')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
        } else if l.trim_start().starts_with('(') {
            if let Some((_, val)) = l.split_once("REG_SZ") {
                let val = val.trim().trim_matches('"');
                if !cle.is_empty() && !val.is_empty() {
                    v.push((cle.clone(), val.to_string()));
                }
            }
        }
    }
    v
}

/// Navigateurs trouvés. `env` lit une variable, `existe` teste un fichier, `reg` = sortie des
/// App Paths (Windows) ; `windows` choisit les règles (testable sur les deux systèmes).
pub fn trouver(
    env: &dyn Fn(&str) -> Option<String>,
    existe: &dyn Fn(&Path) -> bool,
    reg: &str,
    windows: bool,
) -> Vec<Navigateur> {
    let inscrits = app_paths(reg);
    let mut v = Vec::new();
    for (nom, page, chemins, exe_reg, linux) in MODELES {
        let mut candidats: Vec<PathBuf> = Vec::new();
        if windows {
            for base in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
                if let Some(b) = env(base) {
                    for c in chemins {
                        candidats.push(PathBuf::from(format!("{}\\{c}", b.trim_end_matches('\\'))));
                    }
                }
            }
            candidats.extend(
                inscrits
                    .iter()
                    .filter(|(e, _)| !exe_reg.is_empty() && e == exe_reg)
                    .map(|(_, c)| PathBuf::from(c)),
            );
        } else if let Some(path) = env("PATH") {
            for d in path.split(':').filter(|d| !d.is_empty()) {
                candidats.extend(linux.iter().map(|n| Path::new(d).join(n)));
            }
        }
        if let Some(exe) = candidats.into_iter().find(|c| existe(c)) {
            v.push(Navigateur {
                nom,
                exe,
                page,
                firefox: nom == "Firefox",
            });
        }
    }
    v
}

/// Navigateurs installés sur ce système.
pub fn detecter() -> Vec<Navigateur> {
    let env = |k: &str| std::env::var(k).ok();
    trouver(&env, &|p| p.is_file(), &reg_app_paths(), cfg!(windows))
}

#[cfg(windows)]
fn reg_app_paths() -> String {
    let mut s = String::new();
    for racine in ["HKLM", "HKCU"] {
        let cle = format!(r"{racine}\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths");
        if let Ok(o) = crate::systeme::commande(&crate::systeme::outil("reg"))
            .args(["query", &cle, "/s"])
            .output()
        {
            s += &String::from_utf8_lossy(&o.stdout);
        }
    }
    s
}

#[cfg(not(windows))]
fn reg_app_paths() -> String {
    String::new()
}

/// Ouvre la page des extensions du navigateur (adresse interne passée en argument).
pub fn ouvrir(n: &Navigateur) -> bool {
    // Chemin complet détecté ; dossier courant sûr.
    std::process::Command::new(&n.exe)
        .current_dir(crate::systeme::dossier_sur())
        .arg(n.page)
        .spawn()
        .is_ok()
}

/// Dossier de l'extension décompressée : `%LOCALAPPDATA%\fluxdl\extension` sous Windows.
pub fn dossier() -> PathBuf {
    let local = cfg!(windows)
        .then(|| std::env::var_os("LOCALAPPDATA"))
        .flatten();
    match local {
        Some(d) => PathBuf::from(d).join(dlcore::APP_NAME).join("extension"),
        None => crate::store::data_dir().join("extension"),
    }
}

/// Écrit les fichiers de l'extension (manifeste Chrome) dans `d`, en écrasant les anciens.
pub fn extraire(d: &Path) -> io::Result<()> {
    for (nom, octets) in FICHIERS {
        let f = d.join(nom);
        if let Some(p) = f.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(f, octets)?;
    }
    Ok(())
}

/// Au lancement : un dossier déjà extrait d'une autre version est remis à jour (le navigateur
/// le relit à « Recharger » ou au redémarrage). Pas de dossier = rien à faire.
pub fn mettre_a_jour(d: &Path) {
    let manifeste = FICHIERS
        .iter()
        .find(|f| f.0 == "manifest.json")
        .map(|f| f.1);
    if let (Ok(actuel), Some(m)) = (std::fs::read(d.join("manifest.json")), manifeste) {
        if actuel != m {
            let _ = extraire(d);
        }
    }
}

/// CRC-32 (zip), bit à bit : quelques centaines de Ko, une fois par clic.
fn crc32(d: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in d {
        c ^= b as u32;
        for _ in 0..8 {
            c = (c >> 1) ^ (0xEDB8_8320 & (c & 1).wrapping_neg());
        }
    }
    !c
}

/// Zip Firefox (manifeste Firefox), sans compression, date fixe : identique à chaque appel.
pub fn zip_firefox() -> Vec<u8> {
    let mut entrees: Vec<(&str, &[u8])> = vec![("manifest.json", MANIFESTE_FIREFOX)];
    entrees.extend(FICHIERS.iter().filter(|f| f.0 != "manifest.json").copied());
    let (mut z, mut central) = (Vec::new(), Vec::new());
    for (nom, d) in &entrees {
        let (crc, n, decalage) = (crc32(d), d.len() as u32, z.len() as u32);
        // Version 2.0, pas de drapeau, stocké, 1980-01-01 00:00.
        let commun = |v: &mut Vec<u8>| {
            v.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0x21, 0]);
            v.extend_from_slice(&crc.to_le_bytes());
            v.extend_from_slice(&n.to_le_bytes());
            v.extend_from_slice(&n.to_le_bytes());
            v.extend_from_slice(&(nom.len() as u16).to_le_bytes());
            v.extend_from_slice(&[0, 0]);
        };
        z.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        commun(&mut z);
        z.extend_from_slice(nom.as_bytes());
        z.extend_from_slice(d);
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&[20, 0]);
        commun(&mut central);
        // Commentaire, disque, attributs internes (2 + 2 + 2) et externes (4).
        central.extend_from_slice(&[0; 10]);
        central.extend_from_slice(&decalage.to_le_bytes());
        central.extend_from_slice(nom.as_bytes());
    }
    let (debut, taille) = (z.len() as u32, central.len() as u32);
    z.extend_from_slice(&central);
    z.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    z.extend_from_slice(&[0; 4]);
    z.extend_from_slice(&(entrees.len() as u16).to_le_bytes());
    z.extend_from_slice(&(entrees.len() as u16).to_le_bytes());
    z.extend_from_slice(&taille.to_le_bytes());
    z.extend_from_slice(&debut.to_le_bytes());
    z.extend_from_slice(&[0, 0]);
    z
}

/// Écrit `fluxdl-firefox-<version>.zip` dans `d` ; renvoie son chemin.
pub fn ecrire_zip_firefox(d: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(d)?;
    let f = d.join(format!(
        "{}-firefox-{}.zip",
        dlcore::APP_NAME,
        env!("CARGO_PKG_VERSION")
    ));
    dlcore::engine::write_atomic(&f, &zip_firefox())?;
    Ok(f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const REG: &str = "\r
HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\chrome.exe\r
    (par défaut)    REG_SZ    D:\\Apps\\Chrome\\chrome.exe\r
    Path    REG_SZ    D:\\Apps\\Chrome\r
\r
HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\firefox.exe\r
    (Default)    REG_SZ    \"E:\\Mozilla\\firefox.exe\"\r
\r
HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\vide.exe\r
";

    #[test]
    fn app_paths_lus() {
        assert_eq!(
            app_paths(REG),
            vec![
                ("chrome.exe".into(), r"D:\Apps\Chrome\chrome.exe".into()),
                ("firefox.exe".into(), r"E:\Mozilla\firefox.exe".into()),
            ]
        );
    }

    #[test]
    fn detection_windows() {
        let vars: HashMap<&str, &str> = [
            ("ProgramFiles", r"C:\Program Files"),
            ("ProgramFiles(x86)", r"C:\Program Files (x86)"),
            ("LOCALAPPDATA", r"C:\Users\a\AppData\Local"),
        ]
        .into();
        let env = |k: &str| vars.get(k).map(|v| v.to_string());
        let presents = [
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            r"C:\Users\a\AppData\Local\BraveSoftware\Brave-Browser\Application\brave.exe",
            r"D:\Apps\Chrome\chrome.exe",
            r"E:\Mozilla\firefox.exe",
        ];
        let existe = |p: &Path| presents.contains(&p.to_str().unwrap());
        let n = trouver(&env, &existe, REG, true);
        let noms: Vec<_> = n.iter().map(|n| (n.nom, n.exe.to_str().unwrap())).collect();
        assert_eq!(
            noms,
            vec![
                ("Chrome", presents[2]),
                ("Edge", presents[0]),
                ("Brave", presents[1]),
                ("Firefox", presents[3]),
            ]
        );
        assert!(n[3].firefox && !n[0].firefox);
        assert_eq!(n[1].page, "edge://extensions");
        // Rien d'installé.
        assert!(trouver(&env, &|_| false, "", true).is_empty());
    }

    #[test]
    fn detection_linux() {
        let env = |k: &str| (k == "PATH").then(|| "/usr/local/bin::/usr/bin".to_string());
        let presents = [
            "/usr/bin/chromium",
            "/usr/bin/firefox-esr",
            "/usr/local/bin/brave",
        ];
        let existe = |p: &Path| presents.contains(&p.to_str().unwrap());
        let n = trouver(&env, &existe, REG, false);
        let noms: Vec<_> = n.iter().map(|n| (n.nom, n.exe.to_str().unwrap())).collect();
        assert_eq!(
            noms,
            vec![
                ("Brave", presents[2]),
                ("Firefox", presents[1]),
                ("Chromium", presents[0]),
            ]
        );
    }

    #[test]
    fn extraction_et_mise_a_jour() {
        let d = std::env::temp_dir().join(format!("fluxdl-ext-{}", std::process::id()));
        extraire(&d).unwrap();
        for (nom, octets) in FICHIERS {
            assert_eq!(&std::fs::read(d.join(nom)).unwrap(), octets, "{nom}");
        }
        let m: serde_json::Value =
            serde_json::from_slice(&std::fs::read(d.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(m["version"], env!("CARGO_PKG_VERSION"));
        assert!(m["background"]["service_worker"].is_string());
        assert!(d.join("_locales/fr/messages.json").is_file());
        assert!(d.join("icons/icon128.png").is_file());
        assert!(!d.join("package.sh").exists() && !d.join("dist").exists());
        // Ancienne version : remise à jour ; fichier modifié ailleurs que le manifeste : gardé.
        std::fs::write(d.join("manifest.json"), "{}").unwrap();
        std::fs::write(d.join("popup.js"), "x").unwrap();
        mettre_a_jour(&d);
        assert_eq!(
            std::fs::read_to_string(d.join("popup.js")).unwrap(),
            String::from_utf8_lossy(FICHIERS.iter().find(|f| f.0 == "popup.js").unwrap().1)
        );
        // Pas de dossier : rien n'est créé.
        let absent = d.join("absent");
        mettre_a_jour(&absent);
        assert!(!absent.exists());
        let z = ecrire_zip_firefox(&d).unwrap();
        assert!(z.ends_with(format!("fluxdl-firefox-{}.zip", env!("CARGO_PKG_VERSION"))));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn zip_lisible() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        let z = zip_firefox();
        assert_eq!(z, zip_firefox());
        // Fin du répertoire central : nombre d'entrées = fichiers embarqués.
        let fin = z.len() - 22;
        assert_eq!(&z[fin..fin + 4], &0x0605_4b50u32.to_le_bytes());
        let n = u16::from_le_bytes([z[fin + 10], z[fin + 11]]) as usize;
        assert_eq!(n, FICHIERS.len());
        // Répertoire central : chaque entrée pointe vers son en-tête local, même nom.
        let u32_ = |i: usize| u32::from_le_bytes([z[i], z[i + 1], z[i + 2], z[i + 3]]) as usize;
        let u16_ = |i: usize| u16::from_le_bytes([z[i], z[i + 1]]) as usize;
        let mut c = u32_(fin + 16);
        for _ in 0..n {
            assert_eq!(u32_(c), 0x0201_4b50);
            let (l, local) = (u16_(c + 28), u32_(c + 42));
            assert_eq!(u32_(local), 0x0403_4b50);
            assert_eq!(&z[c + 46..c + 46 + l], &z[local + 30..local + 30 + l]);
            c += 46 + l;
        }
        assert_eq!(c, fin);
        // Première entrée : manifest.json, le manifeste Firefox stocké tel quel.
        let l = u16::from_le_bytes([z[26], z[27]]) as usize;
        assert_eq!(&z[30..30 + l], b"manifest.json");
        assert_eq!(
            &z[30 + l..30 + l + MANIFESTE_FIREFOX.len()],
            MANIFESTE_FIREFOX
        );
    }
}
