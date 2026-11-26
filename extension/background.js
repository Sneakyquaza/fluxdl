// Arrière-plan : menu contextuel, capture auto, repli navigateur.
"use strict";

// Chrome (service worker) charge common.js ici ; Firefox le liste dans le manifeste
if (typeof importScripts === "function") importScripts("common.js");

const MENU_ID = "fluxdl-download";

// Téléchargements lancés par l'extension elle-même (repli) : à ne pas capturer
const ownUrls = new Set();
const ownIds = new Set();

function notify(message) {
  api.notifications.create({
    type: "basic",
    iconUrl: api.runtime.getURL("icons/icon48.png"),
    title: "fluxdl",
    message,
  });
}

async function setupMenu() {
  try {
    await api.contextMenus.removeAll();
  } catch {
    // rien à retirer
  }
  api.contextMenus.create({
    id: MENU_ID,
    title: t("menuDownload"),
    contexts: ["link", "video", "audio"],
  });
}

api.runtime.onInstalled.addListener(setupMenu);
api.runtime.onStartup?.addListener(setupMenu);

// Cookies du site, si l'option et la permission sont là
async function cookiesFor(url, settings) {
  if (!settings.sendCookies || !api.cookies) return undefined;
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

async function sendToApp(url, referrer, filename, settings) {
  const body = { url };
  if (isHttp(referrer)) body.referrer = referrer;
  if (filename) body.filename = filename;
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

api.contextMenus.onClicked.addListener(async (info) => {
  if (info.menuItemId !== MENU_ID) return;
  const url = info.linkUrl || info.srcUrl;
  if (!isHttp(url)) {
    notify(t("notifUnsupported"));
    return;
  }
  const settings = await getSettings();
  const res = await sendToApp(url, info.pageUrl, undefined, settings);
  if (res.state === "ok") return;
  if (["down", "unpaired", "noaccess"].includes(res.state)) {
    await browserDownload(url);
    notify(t("notifNotRunning"));
    return;
  }
  notify(errorMessage(res));
});

function basename(path) {
  return (path || "").split(/[\\/]/).pop();
}

function extOf(name) {
  const clean = (name || "").split(/[?#]/)[0];
  const base = basename(clean);
  const dot = base.lastIndexOf(".");
  return dot > 0 ? base.slice(dot + 1).toLowerCase() : "";
}

function shouldCapture(item, settings) {
  const size = item.totalBytes > 0 ? item.totalBytes : item.fileSize > 0 ? item.fileSize : 0;
  if (size && size >= settings.thresholdMB * 1024 * 1024) return true;
  const exts = settings.extensions
    .split(",")
    .map((e) => e.trim().replace(/^\./, "").toLowerCase())
    .filter(Boolean);
  if (!exts.length) return false;
  let urlPath = "";
  try {
    urlPath = new URL(item.finalUrl || item.url).pathname;
  } catch {
    // URL illisible
  }
  return [extOf(item.filename), extOf(urlPath)].some((e) => e && exts.includes(e));
}

api.downloads.onCreated.addListener(async (item) => {
  const url = item.finalUrl || item.url;
  if (ownIds.has(item.id) || ownUrls.has(item.url) || ownUrls.has(url)) {
    ownUrls.delete(item.url);
    ownUrls.delete(url);
    ownIds.delete(item.id);
    return;
  }
  if (!isHttp(url)) return;
  const settings = await getSettings();
  if (!settings.autoCapture || !shouldCapture(item, settings)) return;
  // Envoi d'abord : si l'appli refuse, le téléchargement continue dans le navigateur
  const res = await sendToApp(url, item.referrer, basename(item.filename) || undefined, settings);
  if (res.state !== "ok") {
    notify(res.state === "rate" ? t("notifRateLimited") : t("notifKeptInBrowser"));
    return;
  }
  try {
    await api.downloads.cancel(item.id);
    await api.downloads.erase({ id: item.id });
  } catch {
    // déjà fini ou supprimé
  }
});
