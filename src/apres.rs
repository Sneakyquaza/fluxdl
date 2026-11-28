//! Après le téléchargement : tri par type (dossier choisi avant d'écrire, jamais de déplacement
//! après coup), extraction des archives zip / 7z, empreinte SHA-256 d'un fichier fini.

use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Catégorie de fichier pour le tri automatique.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Categorie {
    Videos,
    Musique,
    Images,
    Documents,
    Archives,
    Programmes,
    Autres,
}

pub const CATEGORIES: [Categorie; 7] = [
    Categorie::Videos,
    Categorie::Musique,
    Categorie::Images,
    Categorie::Documents,
    Categorie::Archives,
    Categorie::Programmes,
    Categorie::Autres,
];

/// Extensions (minuscules) de chaque catégorie ; le reste va dans « Autres ».
const EXTENSIONS: [(Categorie, &[&str]); 6] = [
    (
        Categorie::Videos,
        &[
            "mp4", "mkv", "webm", "avi", "mov", "wmv", "flv", "m4v", "mpg", "mpeg", "ts", "m2ts",
            "3gp", "ogv",
        ],
    ),
    (
        Categorie::Musique,
        &[
            "mp3", "m4a", "aac", "flac", "wav", "ogg", "opus", "wma", "aiff", "ape", "mka", "alac",
        ],
    ),
    (
        Categorie::Images,
        &[
            "jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff", "svg", "heic", "heif",
            "avif", "ico", "psd",
        ],
    ),
    (
        Categorie::Documents,
        &[
            "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "txt",
            "md", "csv", "epub", "mobi", "djvu",
        ],
    ),
    (
        Categorie::Archives,
        &[
            "zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "tbz2", "xz", "txz", "zst", "lz", "cab",
        ],
    ),
    (
        Categorie::Programmes,
        &[
            "exe", "msi", "msix", "appx", "apk", "deb", "rpm", "appimage", "dmg", "pkg", "iso",
            "img", "jar",
        ],
    ),
];

impl Categorie {
    /// Nom anglais (clé de traduction), aussi nom du dossier par défaut une fois traduit.
    pub fn nom(self) -> &'static str {
        match self {
            Categorie::Videos => "Videos",
            Categorie::Musique => "Music",
            Categorie::Images => "Pictures",
            Categorie::Documents => "Documents",
            Categorie::Archives => "Archives",
            Categorie::Programmes => "Programs",
            Categorie::Autres => "Other",
        }
    }
}

/// Catégorie d'après l'extension du nom (« .tar.gz » → la dernière, gz).
pub fn categorie(nom: &str) -> Categorie {
    let ext = match nom.rsplit_once('.') {
        Some((b, e)) if !b.is_empty() => e.to_ascii_lowercase(),
        _ => return Categorie::Autres,
    };
    EXTENSIONS
        .iter()
        .find(|(_, l)| l.contains(&ext.as_str()))
        .map_or(Categorie::Autres, |(c, _)| *c)
}

/// Avancement d'une extraction ou d'un calcul : octets faits / total, arrêt demandé.
#[derive(Default)]
pub struct Suivi {
    pub fait: AtomicU64,
    pub total: AtomicU64,
    pub stop: AtomicBool,
}

/// Arrêt demandé (rien ne reste sur le disque).
pub const ARRETE: &str = "Stopped.";
pub const ERR_RAR: &str = "RAR archives are not supported.";
pub const ERR_FORMAT: &str = "Only zip and 7z archives can be extracted.";
pub const ERR_CHEMIN: &str =
    "Archive refused: a file path would leave the folder (absolute or ..).";
pub const ERR_CHIFFRE: &str = "Encrypted archive: not supported.";
pub const ERR_CORROMPU: &str = "Damaged archive: a file is larger than announced.";

/// Archive reconnue à son extension.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Format {
    Zip,
    SeptZ,
    Rar,
}

pub fn format(nom: &str) -> Option<Format> {
    let n = nom.to_ascii_lowercase();
    if n.ends_with(".zip") {
        Some(Format::Zip)
    } else if n.ends_with(".7z") {
        Some(Format::SeptZ)
    } else if n.ends_with(".rar") {
        Some(Format::Rar)
    } else {
        None
    }
}

/// Chemin relatif sûr d'une entrée d'archive (séparateurs `/` ou `\`), sinon `None` : absolu,
/// lecteur, `..`, composant vide ou interdit (mêmes règles que les torrents).
fn chemin_sur(nom: &str) -> Option<PathBuf> {
    let nom = nom.strip_suffix(['/', '\\']).unwrap_or(nom);
    let mut p = PathBuf::new();
    for c in nom.split(['/', '\\']) {
        if !crate::torrent::composant_sur(c, cfg!(windows)) {
            return None;
        }
        p.push(c);
    }
    (!p.as_os_str().is_empty()).then_some(p)
}

/// Entrée d'archive lue avant d'écrire quoi que ce soit.
struct Entree {
    chemin: PathBuf,
    dossier: bool,
    taille: u64,
}

/// Extrait `archive` dans un dossier neuf à son nom, à côté (« nom (1) » s'il existe déjà) ;
/// renvoie ce dossier. Tous les chemins sont vérifiés avant d'écrire ; liens ignorés (jamais
/// créés) ; jamais d'écrasement (doublon dans l'archive → « nom (1).ext »). Arrêt ou erreur :
/// le dossier créé est effacé.
pub fn extraire(archive: &Path, suivi: &Suivi) -> Result<PathBuf, String> {
    let nom = archive
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let fmt = format(&nom).ok_or(ERR_FORMAT)?;
    if fmt == Format::Rar {
        return Err(ERR_RAR.into());
    }
    let parent = archive.parent().unwrap_or(Path::new("."));
    let base = match nom.rsplit_once('.') {
        Some((b, _)) if !b.is_empty() => b.to_string(),
        _ => "archive".to_string(),
    };
    match fmt {
        Format::Zip => {
            let f = File::open(archive).map_err(|e| e.to_string())?;
            let mut z = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
            let mut entrees = Vec::new();
            for i in 0..z.len() {
                let f = z.by_index_raw(i).map_err(|e| e.to_string())?;
                if f.encrypted() {
                    return Err(ERR_CHIFFRE.into());
                }
                let n = f.name().map_err(|e| e.to_string())?;
                let chemin = chemin_sur(&n).ok_or(ERR_CHEMIN)?;
                entrees.push((!f.is_symlink()).then_some(Entree {
                    chemin,
                    dossier: f.is_dir(),
                    taille: f.size(),
                }));
            }
            let dest = preparer(parent, &base, entrees.iter().flatten(), suivi)?;
            let r = (|| {
                for (i, e) in entrees.iter().enumerate() {
                    let Some(e) = e else { continue };
                    let mut f = z.by_index(i).map_err(|e| e.to_string())?;
                    ecrire(&dest, e, &mut f, suivi)?;
                }
                Ok(())
            })();
            finir(dest, r)
        }
        _ => {
            let mut z = sevenz_rust2::ArchiveReader::open(archive, sevenz_rust2::Password::empty())
                .map_err(|e| e.to_string())?;
            let entrees = z
                .archive()
                .files
                .iter()
                .map(entree_7z)
                .collect::<Result<Vec<_>, _>>()?;
            let dest = preparer(parent, &base, entrees.iter().flatten(), suivi)?;
            let mut erreur = None;
            let r = z.for_each_entries(|f, lecteur| {
                if let Ok(Some(e)) = entree_7z(f) {
                    if let Err(m) = ecrire(&dest, &e, lecteur, suivi) {
                        erreur = Some(m);
                        return Err(sevenz_rust2::Error::Other("stop".into()));
                    }
                }
                Ok(true)
            });
            let r = match (erreur, r) {
                (Some(m), _) => Err(m),
                (None, r) => r.map_err(|e| e.to_string()),
            };
            finir(dest, r)
        }
    }
}

/// Entrée 7z : chemin vérifié ; lien (jamais créé) ou élément « anti » → `None`.
fn entree_7z(f: &sevenz_rust2::ArchiveEntry) -> Result<Option<Entree>, String> {
    let chemin = chemin_sur(&f.name).ok_or(ERR_CHEMIN)?;
    // Attributs Windows : point d'analyse (lien) ; Unix (bit 0x8000) : S_IFLNK.
    let a = f.windows_attributes;
    let lien = f.has_windows_attributes
        && (a & 0x400 != 0 || (a & 0x8000 != 0 && (a >> 16) & 0o170000 == 0o120000));
    Ok((!lien && !f.is_anti_item).then_some(Entree {
        chemin,
        dossier: f.is_directory,
        taille: f.size,
    }))
}

/// Total annoncé, place libre, dossier de destination neuf.
fn preparer<'a>(
    parent: &Path,
    base: &str,
    entrees: impl Iterator<Item = &'a Entree>,
    suivi: &Suivi,
) -> Result<PathBuf, String> {
    let total: u64 = entrees.map(|e| e.taille).sum();
    suivi.total.store(total, Ordering::Relaxed);
    if dlcore::engine::espace_libre(parent).is_some_and(|l| l < total) {
        return Err(dlcore::engine::ERR_ESPACE.into());
    }
    loop {
        let d = parent.join(dlcore::engine::unique_name(parent, base));
        match fs::create_dir(&d) {
            Ok(()) => return Ok(d),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("{}: {e}", d.display())),
        }
    }
}

/// Dossier ou fichier d'une entrée ; écrit au plus la taille annoncée (archive piégée).
fn ecrire(dest: &Path, e: &Entree, src: &mut dyn Read, suivi: &Suivi) -> Result<(), String> {
    let cible = dest.join(&e.chemin);
    if e.dossier {
        return fs::create_dir_all(&cible).map_err(|e| e.to_string());
    }
    let dir = cible.parent().unwrap_or(dest);
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let nom = cible
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut f = loop {
        let c = dir.join(dlcore::engine::unique_name(dir, &nom));
        match fs::OpenOptions::new().write(true).create_new(true).open(&c) {
            Ok(f) => break f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("{}: {e}", c.display())),
        }
    };
    let mut buf = vec![0u8; 1 << 16];
    let mut n = 0u64;
    loop {
        if suivi.stop.load(Ordering::Relaxed) {
            return Err(ARRETE.into());
        }
        let k = src.read(&mut buf).map_err(|e| e.to_string())?;
        if k == 0 {
            return Ok(());
        }
        n += k as u64;
        if n > e.taille {
            return Err(ERR_CORROMPU.into());
        }
        f.write_all(&buf[..k]).map_err(|e| e.to_string())?;
        suivi.fait.fetch_add(k as u64, Ordering::Relaxed);
    }
}

/// Arrêt ou erreur : le dossier neuf (à nous seuls) est effacé.
fn finir(dest: PathBuf, r: Result<(), String>) -> Result<PathBuf, String> {
    match r {
        Ok(()) => Ok(dest),
        Err(m) => {
            let _ = fs::remove_dir_all(&dest);
            Err(m)
        }
    }
}

/// SHA-256 (hex minuscules) de `p`, avec avancement et arrêt.
pub fn sha256(p: &Path, suivi: &Suivi) -> Result<String, String> {
    let mut f = File::open(p).map_err(|e| e.to_string())?;
    let total = f.metadata().map_err(|e| e.to_string())?.len();
    suivi.total.store(total, Ordering::Relaxed);
    let mut c = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if suivi.stop.load(Ordering::Relaxed) {
            return Err(ARRETE.into());
        }
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        c.update(&buf[..n]);
        suivi.fait.fetch_add(n as u64, Ordering::Relaxed);
    }
    Ok(dlcore::engine::hex(c.finish().as_ref()))
}

/// Empreinte collée égale à celle calculée (espaces et casse ignorés ; « sha256: » accepté).
pub fn meme_empreinte(collee: &str, calculee: &str) -> bool {
    let c = collee.trim().to_ascii_lowercase();
    let c = c.strip_prefix("sha256:").unwrap_or(&c).trim();
    c.eq_ignore_ascii_case(calculee)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn dossier(nom: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("fluxdl-test-apres-{nom}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// Zip en mémoire : (nom, contenu) ; contenu `None` = dossier.
    fn zip(entrees: &[(&str, Option<&[u8]>)]) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (n, c) in entrees {
            match c {
                Some(c) => {
                    z.start_file(*n, o).unwrap();
                    z.write_all(c).unwrap();
                }
                None => z.add_directory(*n, o).unwrap(),
            }
        }
        z.finish().unwrap().into_inner()
    }

    #[test]
    fn categorie_selon_l_extension() {
        assert_eq!(categorie("film.MKV"), Categorie::Videos);
        assert_eq!(categorie("a.flac"), Categorie::Musique);
        assert_eq!(categorie("photo.jpeg"), Categorie::Images);
        assert_eq!(categorie("note.pdf"), Categorie::Documents);
        assert_eq!(categorie("code.tar.gz"), Categorie::Archives);
        assert_eq!(categorie("setup.exe"), Categorie::Programmes);
        assert_eq!(categorie("debian.iso"), Categorie::Programmes);
        assert_eq!(categorie("README"), Categorie::Autres);
        assert_eq!(categorie(".bashrc"), Categorie::Autres);
        assert_eq!(categorie("x.inconnu"), Categorie::Autres);
        // Une catégorie par extension, aucune en double.
        let toutes: Vec<_> = EXTENSIONS.iter().flat_map(|(_, l)| l.iter()).collect();
        let mut uniques = toutes.clone();
        uniques.sort();
        uniques.dedup();
        assert_eq!(toutes.len(), uniques.len());
        assert_eq!(format("A.ZIP"), Some(Format::Zip));
        assert_eq!(format("a.7z"), Some(Format::SeptZ));
        assert_eq!(format("a.rar"), Some(Format::Rar));
        assert_eq!(format("a.tar.gz"), None);
    }

    /// Zip : dossiers et fichiers à côté de l'archive, jamais d'écrasement (« (1) »), archive
    /// gardée ; RAR et format inconnu refusés.
    #[test]
    fn extraction_zip_sans_ecrasement() {
        let d = dossier("zip");
        let a = d.join("paquet.zip");
        fs::write(
            &a,
            zip(&[
                ("docs/", None),
                ("docs/lisez-moi.txt", Some(b"bonjour")),
                ("racine.bin", Some(&[7u8; 300_000])),
                ("vide.txt", Some(b"")),
            ]),
        )
        .unwrap();
        let s = Suivi::default();
        let x = extraire(&a, &s).unwrap();
        assert_eq!(x, d.join("paquet"));
        assert_eq!(fs::read(x.join("docs/lisez-moi.txt")).unwrap(), b"bonjour");
        assert_eq!(fs::read(x.join("racine.bin")).unwrap(), vec![7u8; 300_000]);
        assert_eq!(fs::read(x.join("vide.txt")).unwrap(), b"");
        assert_eq!(s.fait.load(Ordering::Relaxed), 300_007);
        assert_eq!(s.total.load(Ordering::Relaxed), 300_007);
        // Deuxième fois : nouveau dossier, le premier n'est pas touché.
        fs::write(x.join("vide.txt"), b"mien").unwrap();
        let y = extraire(&a, &Suivi::default()).unwrap();
        assert_eq!(y, d.join("paquet (1)"));
        assert_eq!(fs::read(x.join("vide.txt")).unwrap(), b"mien");
        assert!(a.exists());
        // Arrêt demandé : rien ne reste.
        let s = Suivi::default();
        s.stop.store(true, Ordering::Relaxed);
        assert_eq!(extraire(&a, &s).unwrap_err(), ARRETE);
        assert!(!d.join("paquet (2)").exists());
        fs::write(d.join("x.rar"), b"Rar!").unwrap();
        assert_eq!(
            extraire(&d.join("x.rar"), &Suivi::default()).unwrap_err(),
            ERR_RAR
        );
        assert_eq!(
            extraire(&d.join("x.tar"), &Suivi::default()).unwrap_err(),
            ERR_FORMAT
        );
        let _ = fs::remove_dir_all(&d);
    }

    /// Zip-slip : chemin absolu, `..`, lecteur Windows → archive refusée sans rien écrire ;
    /// lien symbolique jamais créé.
    #[test]
    fn zip_slip_refuse() {
        let d = dossier("slip");
        for (k, n) in [
            "../evil.txt",
            "a/../../evil.txt",
            "/tmp/evil.txt",
            "a\\..\\..\\evil.txt",
        ]
        .into_iter()
        .enumerate()
        {
            let a = d.join(format!("piege{k}.zip"));
            fs::write(&a, zip(&[("ok.txt", Some(b"ok")), (n, Some(b"evil"))])).unwrap();
            assert_eq!(
                extraire(&a, &Suivi::default()).unwrap_err(),
                ERR_CHEMIN,
                "{n}"
            );
            assert!(!d.join(format!("piege{k}")).exists(), "{n}");
        }
        assert!(!d.join("evil.txt").exists());
        if cfg!(windows) {
            assert!(chemin_sur("C:/evil.txt").is_none());
        }
        assert!(chemin_sur("a//b").is_none());
        assert_eq!(chemin_sur("a/b/"), Some(Path::new("a").join("b")));
        // Lien : ignoré.
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default();
        z.add_symlink("lien", "/etc/passwd", o).unwrap();
        z.start_file("vrai.txt", o).unwrap();
        z.write_all(b"vrai").unwrap();
        let a = d.join("liens.zip");
        fs::write(&a, z.finish().unwrap().into_inner()).unwrap();
        let x = extraire(&a, &Suivi::default()).unwrap();
        assert!(fs::symlink_metadata(x.join("lien")).is_err());
        assert_eq!(fs::read(x.join("vrai.txt")).unwrap(), b"vrai");
        let _ = fs::remove_dir_all(&d);
    }

    /// 7z (LZMA2) créé dans le test : contenu, sous-dossier, chemin piégé refusé.
    #[test]
    fn extraction_7z() {
        use sevenz_rust2::{ArchiveEntry, ArchiveWriter};
        let d = dossier("7z");
        let ecrire = |chemin: &Path, entrees: &[(&str, &[u8])]| {
            let mut w = ArchiveWriter::create(chemin).unwrap();
            for (n, c) in entrees {
                w.push_archive_entry(ArchiveEntry::new_file(n), Some(*c))
                    .unwrap();
            }
            w.finish().unwrap();
        };
        let a = d.join("jeu.7z");
        let gros: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        ecrire(&a, &[("sous/a.txt", b"alpha"), ("b.bin", &gros)]);
        let x = extraire(&a, &Suivi::default()).unwrap();
        assert_eq!(x, d.join("jeu"));
        assert_eq!(fs::read(x.join("sous/a.txt")).unwrap(), b"alpha");
        assert_eq!(fs::read(x.join("b.bin")).unwrap(), gros);
        let p = d.join("piege.7z");
        ecrire(&p, &[("../evil.txt", b"evil")]);
        assert_eq!(extraire(&p, &Suivi::default()).unwrap_err(), ERR_CHEMIN);
        assert!(!d.join("evil.txt").exists() && !d.join("piege").exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn sha256_connu() {
        let d = dossier("sha");
        let f = d.join("abc.txt");
        fs::write(&f, b"abc").unwrap();
        let s = Suivi::default();
        let h = sha256(&f, &s).unwrap();
        assert_eq!(
            h,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(s.fait.load(Ordering::Relaxed), 3);
        assert!(meme_empreinte(
            "  SHA256:BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD\n",
            &h
        ));
        assert!(!meme_empreinte("ba7816bf", &h));
        let _ = fs::remove_dir_all(&d);
    }
}
