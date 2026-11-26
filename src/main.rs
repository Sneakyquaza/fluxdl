#![cfg_attr(windows, windows_subsystem = "windows")]
//! Gestionnaire de téléchargement multi-connexions : fenêtre egui, ou modes en ligne de
//! commande (`--capture DOSSIER`, `--benchmark URL`).

mod github;
mod i18n;
mod icone;
mod maj;
mod presse;
mod serveur;
mod store;
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
    let capture = valeur("--capture").map(std::path::PathBuf::from);
    // Lien magnet ou .torrent ouvert depuis le navigateur ou l'Explorateur : confié à l'appli
    // déjà lancée s'il y en a une (serveur local, jeton des réglages), sinon ouvert ici.
    let ouvrir = args
        .iter()
        .skip(1)
        .find(|a| a.starts_with("magnet:") || a.to_ascii_lowercase().ends_with(".torrent"));
    if let Some(u) = ouvrir {
        let s: store::Settings = store::load("settings.json").unwrap_or_default();
        let chemin = std::fs::canonicalize(u).map_or(u.clone(), |p| p.display().to_string());
        if s.server && !s.token.is_empty() && serveur::confier(s.port, &s.token, &chemin) {
            return Ok(());
        }
    }
    let ouvrir =
        ouvrir.map(|u| std::fs::canonicalize(u).map_or(u.clone(), |p| p.display().to_string()));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_app_id(APP_NAME)
            .with_inner_size([1180.0, 760.0])
            .with_min_inner_size([1100.0, 680.0])
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
        let ctl = engine::Control::default();
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
