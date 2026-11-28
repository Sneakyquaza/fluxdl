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
  // Sites vidéo (sites.js) : { youtube: true } = cookies de connexion envoyés pour ce site
  videoCookies: {},
};

// Extensions « probablement un fichier » pour le bouton flottant et la page de liens
const FILE_EXTS = (
  "zip,7z,rar,tar,gz,tgz,bz2,xz,zst,iso,img,dmg,exe,msi,msix,appx,apk,deb,rpm,appimage,pkg,bin," +
  "mp4,mkv,webm,avi,mov,m4v,wmv,flv,mp3,m4a,aac,flac,wav,ogg,opus,pdf,epub,torrent"
).split(",");

const APP_ORIGIN = "http://127.0.0.1/*";
const TIMEOUT_MS = 1500;
// /add : l'appli répond après avoir décidé (5 s au plus), et peut d'abord sonder le lien ;
// /add-many : jusqu'à ~1,5 Mo de liens d'une page
const ADD_TIMEOUT_MS = 12000;

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
// state : ok | kept | unpaired | noaccess | down | origin | rate | bad | error
// kept : l'appli a reçu le lien mais ne le télécharge pas (data.status already_done | refused |
// timeout) : le navigateur garde sa copie.
async function callApp(path, body, settings) {
  const s = settings ?? (await getSettings());
  if (!s.token) return { state: "unpaired" };
  if (!(await hasAppAccess())) return { state: "noaccess" };
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), path.startsWith("/add") ? ADD_TIMEOUT_MS : TIMEOUT_MS);
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
    const map = { 200: "ok", 400: "bad", 401: "unpaired", 403: "origin", 409: "kept", 413: "bad", 429: "rate", 503: "kept" };
    let state = map[res.status] ?? "error";
    // Appli 0.2.3+ : « ok » seulement si le lien est ajouté ou déjà en cours (received : liens
    // d'une page reçus, l'appli peut encore demander confirmation)
    if (state === "ok" && typeof data?.status === "string" && !["added", "already_active", "received"].includes(data.status)) {
      state = "kept";
    }
    return { state, status: res.status, data };
  } catch {
    return { state: "down" };
  } finally {
    clearTimeout(timer);
  }
}

// ---- Appairage automatique ----
// POST /pair sans jeton, répété avec le même nonce jusqu'à « Autoriser » / « Refuser » dans
// l'appli (60 s au plus côté appli). L'appli ne donne le jeton qu'à cette demande, une fois.
const PAIR_POLL_MS = 1000;
const PAIR_TIMEOUT_MS = 62000;

async function postPair(port, nonce) {
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), TIMEOUT_MS);
  try {
    const res = await fetch(`http://127.0.0.1:${port}/pair`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ nonce }),
      signal: ctrl.signal,
      cache: "no-store",
      credentials: "omit",
    });
    let data = null;
    try {
      data = await res.json();
    } catch {
      // corps vide
    }
    return { status: res.status, data };
  } catch {
    return { status: 0 };
  } finally {
    clearTimeout(timer);
  }
}

// Résultat : paired | denied | busy | expired | down | noaccess | rate | old | error
async function pairWithApp() {
  const s = await getSettings();
  if (!(await hasAppAccess())) return "noaccess";
  const nonce = Array.from(crypto.getRandomValues(new Uint8Array(16)), (b) => b.toString(16).padStart(2, "0")).join("");
  const end = Date.now() + PAIR_TIMEOUT_MS;
  while (Date.now() < end) {
    const r = await postPair(s.port, nonce);
    if (r.status === 202) {
      await new Promise((ok) => setTimeout(ok, PAIR_POLL_MS));
      // Appel d'API d'extension : garde le service worker éveillé pendant l'attente
      await api.storage.local.get("port");
      continue;
    }
    if (r.status === 200) {
      const token = r.data?.token;
      if (typeof token !== "string" || !/^[0-9a-f]{16,64}$/.test(token)) return "error";
      await saveSettings({ token });
      return "paired";
    }
    const map = { 0: "down", 401: "old", 404: "old", 409: "busy", 410: "expired", 429: "rate" };
    if (r.status === 403) return r.data?.error === "denied" ? "denied" : "error";
    return map[r.status] ?? "error";
  }
  return "expired";
}

// Texte d'un résultat d'appairage
function pairText(state) {
  const keys = {
    paired: "pairPaired",
    denied: "pairDenied",
    busy: "pairBusy",
    expired: "pairExpired",
    down: "statusNotRunning",
    noaccess: "statusNoAccess",
    rate: "notifRateLimited",
    old: "pairOld",
  };
  return t(keys[state] ?? "statusError");
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

// Même site que la page : hôte exact de la page ou un de ses sous-domaines (jamais deux voisins
// sous un même suffixe : a.co.uk ≠ b.co.uk, x.github.io ≠ y.github.io) ;
// raccourci : cdn.exemple.fr depuis www.exemple.fr n'a pas le référent, à revoir si ça gêne.
function sameSite(url, page) {
  const h = hostOf(url);
  const p = hostOf(page);
  return !!p && (h === p || h.endsWith("." + p));
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
