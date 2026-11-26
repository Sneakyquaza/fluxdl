//! Signature des releases (Ed25519, audit M13). Pas dans l'exe publié.
//!
//! `cargo run --example signer -- cle FICHIER.pk8` : crée la clé privée (0600) et affiche la clé
//! publique à recopier dans `src/maj.rs` (`CLE_PUBLIQUE`).
//! `cargo run --example signer -- signer fluxdl.exe VERSION FICHIER.pk8` : écrit `fluxdl.exe.sig`
//! (signature hexadécimale du message `fluxdl.exe VERSION SHA256`).

use ring::signature::{Ed25519KeyPair, KeyPair};

fn hex(o: &[u8]) -> String {
    o.iter().map(|b| format!("{b:02x}")).collect()
}

fn main() -> Result<(), String> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["cle", f] => {
            if std::path::Path::new(f).exists() {
                return Err(format!("{f} existe déjà"));
            }
            let pk8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
                .map_err(|e| e.to_string())?;
            std::fs::write(f, pk8.as_ref()).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(f, std::fs::Permissions::from_mode(0o600))
                    .map_err(|e| e.to_string())?;
            }
            let k = Ed25519KeyPair::from_pkcs8(pk8.as_ref()).map_err(|e| e.to_string())?;
            println!("{}", hex(k.public_key().as_ref()));
            Ok(())
        }
        ["signer", exe, version, cle] => {
            let o = std::fs::read(exe).map_err(|e| e.to_string())?;
            let sha = hex(ring::digest::digest(&ring::digest::SHA256, &o).as_ref());
            let nom = std::path::Path::new(exe)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let k = Ed25519KeyPair::from_pkcs8(&std::fs::read(cle).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            let sig = k.sign(format!("{nom} {version} {sha}").as_bytes());
            std::fs::write(format!("{exe}.sig"), hex(sig.as_ref()) + "\n")
                .map_err(|e| e.to_string())?;
            println!("{exe}.sig ({nom} {version} {sha})");
            Ok(())
        }
        _ => Err("usage : cle FICHIER.pk8 | signer EXE VERSION FICHIER.pk8".into()),
    }
}
