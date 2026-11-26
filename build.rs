//! Windows : identité du programme (VERSIONINFO, nom montré par le Gestionnaire des tâches) et
//! icône de l'exe (dessinée par `src/icone.rs`, PNG dans un .ico), compilées par windres.

include!("src/icone.rs");

/// .ico à entrées PNG (Vista et plus) aux tailles usuelles de l'Explorateur.
fn ecrire_ico(chemin: &std::path::Path) {
    let tailles = [16u32, 24, 32, 48, 64, 128, 256];
    let images: Vec<Vec<u8>> = tailles
        .iter()
        .map(|&n| {
            let mut png = Vec::new();
            let mut enc = png::Encoder::new(&mut png, n, n);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_compression(png::Compression::High);
            let mut w = enc.write_header().expect("en-tête png");
            w.write_image_data(&icone_rgba(n)).expect("png");
            w.finish().expect("png");
            png
        })
        .collect();
    let mut ico = vec![0, 0, 1, 0, tailles.len() as u8, 0];
    let mut decalage = 6 + 16 * tailles.len() as u32;
    for (n, img) in tailles.iter().zip(&images) {
        let c = if *n >= 256 { 0 } else { *n as u8 };
        ico.extend_from_slice(&[c, c, 0, 0, 1, 0, 32, 0]);
        ico.extend_from_slice(&(img.len() as u32).to_le_bytes());
        ico.extend_from_slice(&decalage.to_le_bytes());
        decalage += img.len() as u32;
    }
    for img in &images {
        ico.extend_from_slice(img);
    }
    std::fs::write(chemin, ico).expect("icône .ico");
}

/// Fichiers de l'extension (`extension/`, sans dist/, package.sh, README.md ni le manifeste
/// Firefox), triés : `$OUT_DIR/extension.rs` pour `include!` dans src/extension.rs.
fn embarquer_extension() {
    let racine = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("extension");
    println!("cargo:rerun-if-changed={}", racine.display());
    let mut fichiers = Vec::new();
    let mut pile = vec![racine.clone()];
    while let Some(d) = pile.pop() {
        for e in std::fs::read_dir(&d).expect("extension/") {
            let p = e.expect("extension/").path();
            let rel = p
                .strip_prefix(&racine)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if p.is_dir() {
                if rel != "dist" {
                    println!("cargo:rerun-if-changed={}", p.display());
                    pile.push(p);
                }
            } else if !["package.sh", "README.md", "manifest.firefox.json"].contains(&rel.as_str())
            {
                println!("cargo:rerun-if-changed={}", p.display());
                fichiers.push((rel, p));
            }
        }
    }
    fichiers.sort();
    let mut code = String::from("const FICHIERS: &[(&str, &[u8])] = &[\n");
    for (rel, p) in &fichiers {
        code += &format!(
            "    ({rel:?}, include_bytes!({:?})),\n",
            p.display().to_string()
        );
    }
    let ff = racine.join("manifest.firefox.json");
    println!("cargo:rerun-if-changed={}", ff.display());
    code += &format!(
        "];\nconst MANIFESTE_FIREFOX: &[u8] = include_bytes!({:?});\n",
        ff.display().to_string()
    );
    let sortie = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(sortie.join("extension.rs"), code).expect("extension.rs");
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    embarquer_extension();
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=src/icone.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let sortie = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let nom = env!("CARGO_PKG_NAME");
    let version = env!("CARGO_PKG_VERSION");
    let mut n: Vec<&str> = version.split('.').collect();
    n.resize(4, "0");
    let num = n.join(",");
    let ico = sortie.join("fluxdl.ico");
    ecrire_ico(&ico);
    let ico = ico.display().to_string().replace('\\', "/");
    let texte = format!(
        "1 ICON \"{ico}\"\n\
         1 VERSIONINFO\nFILEVERSION {num}\nPRODUCTVERSION {num}\nBEGIN\n\
         BLOCK \"StringFileInfo\"\nBEGIN\nBLOCK \"040904b0\"\nBEGIN\n\
         VALUE \"CompanyName\", \"Sneakyquaza\"\n\
         VALUE \"ProductName\", \"{nom}\"\n\
         VALUE \"FileDescription\", \"{nom} download manager\"\n\
         VALUE \"InternalName\", \"{nom}\"\n\
         VALUE \"OriginalFilename\", \"{nom}.exe\"\n\
         VALUE \"FileVersion\", \"{version}\"\n\
         VALUE \"ProductVersion\", \"{version}\"\n\
         END\nEND\nBLOCK \"VarFileInfo\"\nBEGIN\nVALUE \"Translation\", 0x409, 1200\nEND\nEND\n"
    );
    let rc = sortie.join("ressources.rc");
    std::fs::write(&rc, texte).expect("ressources.rc");
    let obj = sortie.join("ressources.o");
    let windres = std::env::var("WINDRES").unwrap_or_else(|_| {
        if cfg!(windows) {
            "windres".into()
        } else {
            "x86_64-w64-mingw32-windres".into()
        }
    });
    let ok = std::process::Command::new(&windres)
        .arg(&rc)
        .args(["-O", "coff", "-o"])
        .arg(&obj)
        .status()
        .unwrap_or_else(|e| panic!("{windres} introuvable ({e}) : WINDRES pour le chemin"));
    assert!(ok.success(), "{windres} a échoué");
    println!("cargo:rustc-link-arg-bins={}", obj.display());
}
