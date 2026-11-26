//! Windows : identité du programme (VERSIONINFO, nom montré par le Gestionnaire des tâches),
//! compilée par windres.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=Cargo.toml");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let sortie = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let nom = env!("CARGO_PKG_NAME");
    let version = env!("CARGO_PKG_VERSION");
    let mut n: Vec<&str> = version.split('.').collect();
    n.resize(4, "0");
    let num = n.join(",");
    let texte = format!(
        "1 VERSIONINFO\nFILEVERSION {num}\nPRODUCTVERSION {num}\nBEGIN\n\
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
