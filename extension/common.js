// Code partagé : réglages, appel de l'appli, i18n.
"use strict";

const api = globalThis.browser ?? chrome;

const DEFAULTS = {
  port: 47591,
  token: "",
  sendCookies: false,
  autoCapture: false,
  thresholdMB: 20,
  extensions: "zip,7z,rar,iso,exe,msi,dmg,mp4,mkv",
  captureAll: false,
  floatButton: true,
  // Listes de sites (un par ligne ou séparés par virgules ; « exemple.com » couvre ses sous-domaines)
  allowSites: "",
  denySites: "",
  cookieSites: "",
};

// Extensions « probablement un fichier » pour le bouton flottant et la page de liens
const FILE_EXTS = (
  "zip,7z,rar,tar,gz,tgz,bz2,xz,zst,iso,img,dmg,exe,msi,msix,appx,apk,deb,rpm,appimage,pkg,bin," +
  "mp4,mkv,webm,avi,mov,m4v,wmv,flv,mp3,m4a,aac,flac,wav,ogg,opus,pdf,epub,torrent"
).split(",");

const APP_ORIGIN = "http://127.0.0.1/*";
const TIMEOUT_MS = 1500;

function t(key, subs) {
  return api.i18n.getMessage(key, subs) || key;
}

async function getSettings() {
  const s = await api.storage.local.get(DEFAULTS);
  return { ...DEFAULTS, ...s };
}

function saveSettings(part) {
  return api.storage.local.set(part);
}

// Accès hôte à 127.0.0.1 (optionnel dans Firefox MV3)
async function hasAppAccess() {
  try {
    return await api.permissions.contains({ origins: [APP_ORIGIN] });
  } catch {
    return true;
  }
}

// Appel de l'appli ; renvoie { state, status, data }
// state : ok | unpaired | noaccess | down | origin | rate | bad | error
async function callApp(path, body, settings) {
  const s = settings ?? (await getSettings());
  if (!s.token) return { state: "unpaired" };
  if (!(await hasAppAccess())) return { state: "noaccess" };
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), TIMEOUT_MS);
  try {
    const opts = {
      method: body ? "POST" : "GET",
      headers: { "X-Fluxdl-Token": s.token },
      signal: ctrl.signal,
      cache: "no-store",
      credentials: "omit",
    };
    if (body) {
      opts.headers["Content-Type"] = "application/json";
      opts.body = JSON.stringify(body);
    }
    const res = await fetch(`http://127.0.0.1:${s.port}${path}`, opts);
    let data = null;
    try {
      data = await res.json();
    } catch {
      // corps vide ou non JSON
    }
    const map = { 200: "ok", 400: "bad", 401: "unpaired", 403: "origin", 429: "rate" };
    return { state: map[res.status] ?? "error", status: res.status, data };
  } catch {
    return { state: "down" };
  } finally {
    clearTimeout(timer);
  }
}

function ping(settings) {
  return callApp("/ping", null, settings);
}

function isHttp(url) {
  return /^https?:\/\//i.test(url || "");
}

// « a, .b ;c\nd » → ["a", "b", "c", "d"] (minuscules, sans point initial)
function parseList(text) {
  return String(text || "")
    .split(/[\s,;]+/)
    .map((x) => x.trim().replace(/^\./, "").toLowerCase())
    .filter(Boolean);
}

function hostOf(url) {
  try {
    return new URL(url).hostname.toLowerCase();
  } catch {
    return "";
  }
}

// Vrai si l'hôte est un des sites de la liste ou un de leurs sous-domaines
function hostInList(host, listText) {
  if (!host) return false;
  return parseList(listText).some((site) => {
    const s = site.replace(/^\*\./, "").replace(/^https?:\/\//, "").split("/")[0];
    return s && (host === s || host.endsWith("." + s));
  });
}

// Remplit les éléments marqués data-i18n*
function localizePage() {
  document.documentElement.lang = api.i18n.getUILanguage().split("-")[0];
  for (const el of document.querySelectorAll("[data-i18n]")) {
    el.textContent = t(el.dataset.i18n);
  }
  for (const el of document.querySelectorAll("[data-i18n-placeholder]")) {
    el.placeholder = t(el.dataset.i18nPlaceholder);
  }
  for (const el of document.querySelectorAll("[data-i18n-title]")) {
    el.title = t(el.dataset.i18nTitle);
  }
}
