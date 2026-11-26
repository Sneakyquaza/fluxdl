// Arrière-plan : menus contextuels, capture auto, bouton flottant, page de liens, repli navigateur.
// Permissions : voir README.md (le script de contenu sur http(s) est la seule nouveauté de la 0.2.1 :
// bouton flottant + liste des liens ; pas de tabs/scripting/activeTab/webRequest).
// Seul endroit qui parle à l'appli (jeton) : le script de contenu et la page de liens passent par ici.
"use strict";

// Chrome (service worker) charge common.js ici ; Firefox le liste dans le manifeste
if (typeof importScripts === "function") importScripts("common.js");

const MENU_LINK = "fluxdl-download";
const MENU_MEDIA = "fluxdl-media";
const MENU_PAGE = "fluxdl-page";
const MAX_URL = 8192;
const MAX_LINKS = 2000;
const FALLBACK_STATES = ["down", "unpaired", "noaccess"];

// Téléchargements lancés par l'extension elle-même (repli) : à ne pas capturer
const ownUrls = new Set();
const ownIds = new Set();

let lastNotif = 0;
function notify(message, throttle) {
  const now = Date.now();
  if (throttle && now - lastNotif < 8000) return;
  lastNotif = now;
  api.notifications.create({
    type: "basic",
    iconUrl: api.runtime.getURL("icons/icon48.png"),
    title: "fluxdl",
    message,
  });
}

// Badge « ! » sur l'icône : effacé à l'ouverture du popup
function flagBadge() {
  try {
    api.action.setBadgeBackgroundColor({ color: "#d63a2f" });
    api.action.setBadgeText({ text: "!" });
  } catch {
    // action indisponible
  }
}

async function setupMenu() {
  try {
    await api.contextMenus.removeAll();
  } catch {
    // rien à retirer
  }
  api.contextMenus.create({ id: MENU_LINK, title: t("menuDownload"), contexts: ["link"] });
  api.contextMenus.create({ id: MENU_MEDIA, title: t("menuDownloadMedia"), contexts: ["video", "audio"] });
  api.contextMenus.create({
    id: MENU_PAGE,
    title: t("menuDownloadAll"),
    contexts: ["page"],
    documentUrlPatterns: ["http://*/*", "https://*/*"],
  });
}

api.runtime.onInstalled.addListener(setupMenu);
api.runtime.onStartup?.addListener(setupMenu);

// Cookies du site, si l'option, la permission et la liste de sites le permettent
async function cookiesFor(url, settings) {
  if (!settings.sendCookies || !api.cookies) return undefined;
  if (parseList(settings.cookieSites).length && !hostInList(hostOf(url), settings.cookieSites)) {
    return undefined;
  }
  try {
    const ok = await api.permissions.contains({ permissions: ["cookies"], origins: ["<all_urls>"] });
    if (!ok) return undefined;
    const list = await api.cookies.getAll({ url });
    if (!list.length) return undefined;
    return list.map((c) => `${c.name}=${c.value}`).join("; ");
  } catch {
    return undefined;
  }
}

async function sendToApp(url, extra, settings) {
  const body = { url };
  if (isHttp(extra.referrer)) body.referrer = extra.referrer;
  if (extra.filename) body.filename = extra.filename;
  if (extra.title) body.title = extra.title; // ignoré par l'appli 0.2.x, gardé pour plus tard
  const cookies = await cookiesFor(url, settings);
  if (cookies) body.cookies = cookies;
  return callApp("/add", body, settings);
}

// Repli : le navigateur télécharge lui-même
async function browserDownload(url) {
  ownUrls.add(url);
  try {
    const id = await api.downloads.download({ url });
    ownIds.add(id);
  } catch {
    ownUrls.delete(url);
  }
}

function errorMessage(res) {
  if (res.state === "rate") return t("notifRateLimited");
  if (res.state === "origin") return t("statusBadOrigin");
  return t("notifRejected", [String(res.status ?? "?")]);
}

// Envoi manuel (menu, bouton flottant) : repli navigateur si l'appli est absente
async function sendManual(url, extra) {
  const settings = await getSettings();
  const res = await sendToApp(url, extra, settings);
  if (res.state === "ok") return "ok";
  if (FALLBACK_STATES.includes(res.state)) {
    await browserDownload(url);
    notify(t("notifNotRunning"));
    return "browser";
  }
  notify(errorMessage(res));
  return res.state;
}

// ---- Page de liens ----

const pickerMemory = new Map(); // repli si storage.session manque

async function openPicker(tab) {
  let data;
  try {
    data = await api.tabs.sendMessage(tab.id, { type: "fluxdl-collect" });
  } catch {
    notify(t("notifReloadPage"));
    return;
  }
  const links = cleanLinks(data?.links);
  const id = crypto.randomUUID();
  const page = typeof data?.page === "string" && isHttp(data.page) ? data.page : "";
  const entry = { links, page, title: typeof data?.title === "string" ? data.title.slice(0, 300) : "" };
  if (api.storage.session) await api.storage.session.set({ ["pick-" + id]: entry });
  else pickerMemory.set(id, entry);
  await api.tabs.create({ url: api.runtime.getURL("picker.html") + "#" + id, index: tab.index + 1 });
}

function cleanLinks(raw) {
  if (!Array.isArray(raw)) return [];
  const seen = new Set();
  const out = [];
  for (const l of raw) {
    if (!l || typeof l.url !== "string" || l.url.length > MAX_URL || !isHttp(l.url) || seen.has(l.url)) continue;
    seen.add(l.url);
    out.push({ url: l.url, text: typeof l.text === "string" ? l.text.slice(0, 200) : "", media: l.media === true, dl: l.dl === true });
    if (out.length >= MAX_LINKS) break;
  }
  return out;
}

async function pickerData(id) {
  if (typeof id !== "string" || !/^[0-9a-f-]{36}$/.test(id)) return null;
  const key = "pick-" + id;
  if (api.storage.session) {
    const got = await api.storage.session.get(key);
    return got[key] ?? null;
  }
  return pickerMemory.get(id) ?? null;
}

api.contextMenus.onClicked.addListener(async (info, tab) => {
  if (info.menuItemId === MENU_PAGE) {
    if (tab?.id >= 0) openPicker(tab);
    return;
  }
  if (info.menuItemId !== MENU_LINK && info.menuItemId !== MENU_MEDIA) return;
  const url = info.menuItemId === MENU_LINK ? info.linkUrl : info.srcUrl;
  if (!isHttp(url)) {
    notify(t("notifUnsupported"));
    return;
  }
  await sendManual(url, { referrer: info.pageUrl, title: tab?.title });
});

// ---- Messages du script de contenu et de la page de liens ----

function fromOwnPage(sender) {
  return typeof sender.url === "string" && sender.url.startsWith(api.runtime.getURL(""));
}

async function handleMessage(msg, sender) {
  if (sender.id !== api.runtime.id || !msg || typeof msg !== "object") return { error: "bad" };
  const fromTab = !!sender.tab && !fromOwnPage(sender);
  switch (msg.type) {
    case "fluxdl-config": {
      const s = await getSettings();
      return { floatButton: !!s.floatButton, exts: [...new Set([...FILE_EXTS, ...parseList(s.extensions)])] };
    }
    case "fluxdl-add": {
      if (typeof msg.url !== "string" || msg.url.length > MAX_URL || !isHttp(msg.url)) return { error: "url" };
      if (fromTab) {
        // Référent = adresse réelle du cadre (pas celle que la page prétend)
        const title = typeof msg.title === "string" ? msg.title.slice(0, 300) : "";
        return { state: await sendManual(msg.url, { referrer: sender.url, title }) };
      }
      if (!fromOwnPage(sender)) return { error: "sender" };
      // Page de liens : pas de repli navigateur (des dizaines de fichiers), l'état revient à la page
      const referrer = typeof msg.referrer === "string" ? msg.referrer : "";
      const res = await sendToApp(msg.url, { referrer }, await getSettings());
      return { state: res.state, status: res.status };
    }
    case "fluxdl-picker": {
      if (!fromOwnPage(sender)) return { error: "sender" };
      return { data: await pickerData(msg.id) };
    }
    default:
      return { error: "type" };
  }
}

api.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  handleMessage(msg, sender).then(sendResponse, () => sendResponse({ error: "internal" }));
  return true;
});

// ---- Capture automatique ----

function basename(path) {
  return (path || "").split(/[\\/]/).pop();
}

function extOf(name) {
  const clean = (name || "").split(/[?#]/)[0];
  const base = basename(clean);
  const dot = base.lastIndexOf(".");
  return dot > 0 ? base.slice(dot + 1).toLowerCase() : "";
}

function isOwn(item) {
  const url = item.finalUrl || item.url;
  if (item.byExtensionId === api.runtime.id || ownIds.has(item.id) || ownUrls.has(item.url) || ownUrls.has(url)) {
    ownUrls.delete(item.url);
    ownUrls.delete(url);
    ownIds.delete(item.id);
    return true;
  }
  return false;
}

function shouldCapture(item, settings) {
  const url = item.finalUrl || item.url;
  // blob:, data:, file:… : l'appli ne peut pas les télécharger
  if (!isHttp(url)) return false;
  if (item.byExtensionId) return false; // lancé par une autre extension
  if (!settings.autoCapture) return false;
  const hosts = [hostOf(url), hostOf(item.referrer)];
  if (hosts.some((h) => hostInList(h, settings.denySites))) return false;
  if (settings.captureAll || hosts.some((h) => hostInList(h, settings.allowSites))) return true;
  const size = item.totalBytes > 0 ? item.totalBytes : item.fileSize > 0 ? item.fileSize : 0;
  if (size && size >= settings.thresholdMB * 1024 * 1024) return true;
  const exts = parseList(settings.extensions);
  if (!exts.length) return false;
  let urlPath = "";
  try {
    urlPath = new URL(url).pathname;
  } catch {
    // URL illisible
  }
  return [extOf(item.filename), extOf(urlPath)].some((e) => e && exts.includes(e));
}

// Envoie à l'appli ; vrai seulement si l'appli a confirmé (le navigateur doit alors abandonner)
async function handOver(item, settings) {
  const url = item.finalUrl || item.url;
  const res = await sendToApp(url, { referrer: item.referrer, filename: basename(item.filename) || undefined }, settings);
  if (res.state === "ok") return true;
  flagBadge();
  notify(res.state === "rate" ? t("notifRateLimited") : t("notifKeptInBrowser"), true);
  return false;
}

async function dropBrowserCopy(id) {
  try {
    await api.downloads.cancel(id);
  } catch {
    // déjà fini
  }
  try {
    await api.downloads.erase({ id });
  } catch {
    // déjà supprimé
  }
}

if (api.downloads.onDeterminingFilename) {
  // Chrome/Edge : appelé après les redirections (finalUrl connu) et avant que le fichier soit
  // enregistré ; le navigateur attend notre réponse (suggest), l'appli a donc le temps de confirmer.
  api.downloads.onDeterminingFilename.addListener((item, suggest) => {
    if (isOwn(item)) {
      suggest();
      return;
    }
    (async () => {
      const settings = await getSettings();
      if (shouldCapture(item, settings) && (await handOver(item, settings))) {
        await dropBrowserCopy(item.id);
        try {
          suggest();
        } catch {
          // téléchargement déjà annulé
        }
        return;
      }
      suggest();
    })().catch(() => {
      try {
        suggest();
      } catch {
        // rien
      }
    });
    return true; // réponse asynchrone
  });
} else {
  // Firefox : pas d'onDeterminingFilename ; pause pendant la question à l'appli
  api.downloads.onCreated.addListener(async (item) => {
    if (isOwn(item) || item.state !== "in_progress") return;
    const settings = await getSettings();
    if (!shouldCapture(item, settings)) return;
    let paused = false;
    try {
      await api.downloads.pause(item.id);
      paused = true;
    } catch {
      // non suspendable : on continue quand même
    }
    if (await handOver(item, settings)) {
      await dropBrowserCopy(item.id);
    } else if (paused) {
      api.downloads.resume(item.id).catch(() => {});
    }
  });
}
