//! Lancement des programmes du système : toujours par leur chemin complet (Windows cherche un
//! nom nu d'abord dans le dossier de l'exe, souvent Téléchargements) et depuis un dossier
//! courant sûr (jamais le dossier de téléchargement).

use std::path::{Path, PathBuf};
use std::process::Command;

/// `%SystemRoot%` (Windows), `C:\Windows` à défaut.
#[cfg(windows)]
fn racine_systeme() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
}

/// Outil de `System32` par son chemin complet (`reg`, `taskkill`, `tar`…) ; ailleurs le nom
/// (cherché dans le PATH seulement).
pub fn outil(nom: &str) -> PathBuf {
    #[cfg(windows)]
    return racine_systeme().join("System32").join(format!("{nom}.exe"));
    #[cfg(not(windows))]
    PathBuf::from(nom)
}

/// Dossier courant sûr des programmes lancés : le dossier de données de l'appli (créé), sinon
/// `System32` (Windows) ou `/`.
pub fn dossier_sur() -> PathBuf {
    let d = crate::store::data_dir();
    if std::fs::create_dir_all(&d).is_ok() {
        return d;
    }
    #[cfg(windows)]
    return racine_systeme().join("System32");
    #[cfg(not(windows))]
    PathBuf::from("/")
}

/// Commande sans fenêtre de console (Windows), dossier courant sûr.
pub fn commande(prog: &Path) -> Command {
    let mut c = Command::new(prog);
    c.current_dir(dossier_sur());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Pas de fenêtre de console (l'appli est en sous-système « windows »).
        c.creation_flags(0x0800_0000);
    }
    c
}

/// Ouvre un dossier dans l'explorateur de fichiers.
pub fn ouvrir_dossier(d: &Path) {
    #[cfg(windows)]
    let mut c = {
        let mut c = Command::new(racine_systeme().join("explorer.exe"));
        c.current_dir(dossier_sur());
        c
    };
    #[cfg(not(windows))]
    let mut c = commande(Path::new("xdg-open"));
    let _ = c.arg(d).spawn();
}

/// Ouvre une adresse http(s) dans le navigateur par défaut ; toute autre adresse est ignorée
/// (pas de `file:`, `javascript:`, chemin local ou option).
pub fn ouvrir_url(url: &str) -> bool {
    if !url_web(url) {
        return false;
    }
    #[cfg(windows)]
    let mut c = {
        let mut c = Command::new(racine_systeme().join("explorer.exe"));
        c.current_dir(dossier_sur());
        c
    };
    #[cfg(not(windows))]
    let mut c = commande(Path::new("xdg-open"));
    c.arg(url).spawn().is_ok()
}

/// Adresse web ouvrable : http(s)://hôte…, sans espace ni caractère de contrôle.
pub fn url_web(url: &str) -> bool {
    let l = url.to_ascii_lowercase();
    let reste = l
        .strip_prefix("https://")
        .or_else(|| l.strip_prefix("http://"));
    reste.is_some_and(|r| !r.is_empty() && !r.starts_with('/'))
        && !url.chars().any(|c| c.is_control() || c == ' ' || c == '"')
}

/// Veille ou arrêt du PC : outil (`outil`) et arguments, construits sans rien lancer.
pub fn commande_fin(eteindre: bool, windows: bool) -> (&'static str, &'static [&'static str]) {
    match (windows, eteindre) {
        (true, true) => ("shutdown", &["/s", "/t", "0"]),
        (true, false) => ("rundll32", &["powrprof.dll,SetSuspendState", "0,1,0"]),
        (false, true) => ("systemctl", &["poweroff"]),
        (false, false) => ("systemctl", &["suspend"]),
    }
}

/// Lance la veille ou l'arrêt du PC (`App` le reçoit en paramètre : les tests en mettent un faux).
pub fn veille_ou_arret(eteindre: bool) {
    let (nom, args) = commande_fin(eteindre, cfg!(windows));
    if let Err(e) = commande(&outil(nom)).args(args).spawn() {
        eprintln!("fluxdl: {nom}: {e}");
    }
}

/// Minute de la journée à l'heure locale (0 à 1439).
pub fn minute_locale() -> u16 {
    #[cfg(windows)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct SystemTime {
            annee: u16,
            mois: u16,
            jour_semaine: u16,
            jour: u16,
            heure: u16,
            minute: u16,
            seconde: u16,
            ms: u16,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetLocalTime(t: *mut SystemTime);
        }
        let mut t = SystemTime::default();
        unsafe { GetLocalTime(&mut t) };
        t.heure * 60 + t.minute
    }
    #[cfg(not(windows))]
    {
        let maintenant = unsafe { libc::time(std::ptr::null_mut()) };
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        if unsafe { libc::localtime_r(&maintenant, &mut tm) }.is_null() {
            return 0;
        }
        (tm.tm_hour * 60 + tm.tm_min) as u16
    }
}

/// Animations coupées dans le système (Windows : « Afficher les animations dans Windows »
/// désactivé). Linux : non lu (raccourci : réglage GNOME/KDE à lire par gsettings/kreadconfig
/// si on le demande).
pub fn animations_coupees() -> bool {
    #[cfg(windows)]
    {
        const SPI_GETCLIENTAREAANIMATION: u32 = 0x1042;
        #[link(name = "user32")]
        extern "system" {
            fn SystemParametersInfoW(action: u32, param: u32, valeur: *mut i32, ini: u32) -> i32;
        }
        let mut on: i32 = 1;
        let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, &mut on, 0) };
        ok != 0 && on == 0
    }
    #[cfg(not(windows))]
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seules_les_adresses_web_s_ouvrent() {
        assert!(url_web("https://example.com/a?b=c"));
        assert!(url_web("HTTP://example.com"));
        for u in [
            "file:///C:/Windows/System32/calc.exe",
            "javascript:alert(1)",
            r"C:\Users\x\Downloads\explorer.exe",
            "/select,C:\\x",
            "https://",
            "https:///x",
            "https://a b",
            "https://a\"/x",
            "https://a\n",
        ] {
            assert!(!url_web(u), "{u}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn outils_par_chemin_complet() {
        for n in ["reg", "taskkill", "tar"] {
            let p = outil(n);
            assert!(p.is_absolute());
            assert!(p.ends_with(format!("System32\\{n}.exe")));
        }
    }

    /// 0.2.9 : veille et arrêt du PC construits sans être lancés.
    #[test]
    fn commandes_de_fin() {
        assert_eq!(
            commande_fin(true, true),
            ("shutdown", &["/s", "/t", "0"][..])
        );
        assert_eq!(
            commande_fin(false, true),
            ("rundll32", &["powrprof.dll,SetSuspendState", "0,1,0"][..])
        );
        assert_eq!(commande_fin(true, false), ("systemctl", &["poweroff"][..]));
        assert_eq!(commande_fin(false, false), ("systemctl", &["suspend"][..]));
        assert!(minute_locale() < 1440);
    }

    #[test]
    fn dossier_courant_pas_le_dossier_de_telechargement() {
        let c = commande(Path::new("x"));
        let d = c.get_current_dir().unwrap();
        assert!(d.is_absolute());
        assert_eq!(d, crate::store::data_dir());
    }
}
