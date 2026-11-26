#![cfg_attr(windows, windows_subsystem = "windows")]
//! Gestionnaire de téléchargement multi-connexions : fenêtre egui, ou modes en ligne de
//! commande (`--capture DOSSIER`, `--benchmark URL`).

mod cookies;
mod extension;
mod github;
mod i18n;
mod icone;
mod instance;
mod maj;
mod motw;
mod presse;
mod serveur;
mod store;
mod systeme;
mod torrent;
mod ui;
mod video;

use dlcore::{engine, APP_NAME};
use eframe::egui;
use std::time::Duration;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let valeur = |k: &str| {
        args.iter()
            .position(|a| a == k)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    if let Some(url) = valeur("--benchmark") {
        return benchmark(&url, &args);
    }
    // Chemins rendus absolus avant de quitter le dossier courant.
    let capture = valeur("--capture").map(|d| std::path::absolute(&d).unwrap_or(d.into()));
    // Lien (magnet, http(s)) ou .torrent ouvert depuis le navigateur ou l'Explorateur.
    let ouvrir = args
        .iter()
        .skip(1)
        .find(|a| {
            let b = a.to_ascii_lowercase();
            b.starts_with("magnet:")
                || b.starts_with("http://")
                || b.starts_with("https://")
                || b.ends_with(".torrent")
        })
        .map(|u| std::fs::canonicalize(u).map_or(u.clone(), |p| p.display().to_string()));
    // Instance unique : déjà lancée → le lien lui est confié (ou sa fenêtre montrée), et on
    // s'arrête. Le serveur local écoute toujours la 2e instance (jeton des réglages).
    let _verrou = if capture.is_none() {
        let donnees = store::data_dir();
        let verrou = if args.iter().any(|a| a == maj::APRES_MAJ) {
            instance::attendre(&donnees, Duration::from_secs(15))
        } else {
            instance::prendre(&donnees)
        };
        if verrou.is_none() {
            // La 1re instance peut être en train de démarrer : quelques essais.
            let fin = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                let s: store::Settings = store::load("settings.json").unwrap_or_default();
                let ok = !s.token.is_empty()
                    && match &ouvrir {
                        Some(u) => serveur::confier(s.port, &s.token, u),
                        None => serveur::montrer(s.port, &s.token),
                    };
                if ok || std::time::Instant::now() >= fin {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(300));
            }
        }
        // Dossier courant sûr : jamais le dossier de lancement (souvent Téléchargements).
        let _ = std::env::set_current_dir(systeme::dossier_sur());
        verrou
    } else {
        None
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_app_id(APP_NAME)
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([900.0, 600.0])
            .with_icon(egui::IconData {
                rgba: icone::icone_rgba(128),
                width: 128,
                height: 128,
            }),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| Ok(Box::new(ui::App::new(&cc.egui_ctx, capture, ouvrir)))),
    )
    .map_err(|e| e.to_string())
}

/// `--benchmark URL [--connections 1,8,16] [--seconds 10]` : débit moyen par nombre de
/// connexions, fichiers temporaires effacés.
fn benchmark(url: &str, args: &[String]) -> Result<(), String> {
    let valeur = |k: &str| {
        args.iter()
            .position(|a| a == k)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let liste = valeur("--connections").unwrap_or_else(|| "1,8,16".into());
    let secs: u64 = valeur("--seconds")
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    for n in liste
        .split(',')
        .filter_map(|n| n.trim().parse::<usize>().ok())
    {
        let ctl = std::sync::Arc::new(engine::Control::default());
        let v = engine::measure(url, n, Duration::from_secs(secs), &ctl)?;
        let l = ctl.snapshot();
        println!(
            "{n:>2} connections : {:>7.1} MB/s  ({} MB, {} steals)",
            v / 1e6,
            l.received / 1_000_000,
            l.steals
        );
    }
    Ok(())
}
