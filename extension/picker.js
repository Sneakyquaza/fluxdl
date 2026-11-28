// Page de choix des liens d'une page : filtres par type, par extension et par texte, cases à
// cocher, envoi groupé à l'appli (/add-many ; appli plus ancienne : un par un).
"use strict";

const $ = (id) => document.getElementById(id);
let links = []; // { url, text, media, dl, name, host, ext, type, likely, checked, state }
let referrer = "";
let sending = false;
let type = "all";

// Mêmes extensions que les catégories de l'appli (src/apres.rs, EXTENSIONS)
const TYPES = {
  videos: "mp4,mkv,webm,avi,mov,wmv,flv,m4v,mpg,mpeg,ts,m2ts,3gp,ogv",
  music: "mp3,m4a,aac,flac,wav,ogg,opus,wma,aiff,ape,mka,alac",
  images: "jpg,jpeg,png,gif,webp,bmp,tif,tiff,svg,heic,heif,avif,ico,psd",
  archives: "zip,7z,rar,tar,gz,tgz,bz2,tbz2,xz,txz,zst,lz,cab",
  documents: "pdf,doc,docx,xls,xlsx,ppt,pptx,odt,ods,odp,rtf,txt,md,csv,epub,mobi,djvu",
  programs: "exe,msi,msix,appx,apk,deb,rpm,appimage,dmg,pkg,iso,img,jar",
};
const TYPE_KEYS = { all: "pickTypeAll", videos: "pickTypeVideos", music: "pickTypeMusic", images: "pickTypeImages", archives: "pickTypeArchives", documents: "pickTypeDocuments", programs: "pickTypePrograms" };
const typeOfExt = (ext) => Object.keys(TYPES).find((k) => TYPES[k].split(",").includes(ext)) ?? "";

// Nom de fichier deviné : dernier morceau du chemin (magnet : dn=), sinon l'hôte
function nameOf(u) {
  try {
    const x = new URL(u);
    if (x.protocol === "magnet:") return new URLSearchParams(x.search).get("dn") || "magnet";
    return decodeURIComponent(x.pathname.split("/").pop() || "") || x.hostname;
  } catch {
    return u;
  }
}

function extOfName(name) {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase().slice(0, 8) : "";
}

// Champ extensions : « .zip, pdf » (fin du nom) ou motif « *.part*.rar » (* et ?)
function extMatcher(text) {
  const parts = String(text || "").split(/[\s,;]+/).map((x) => x.trim().toLowerCase()).filter(Boolean);
  if (!parts.length) return () => true;
  const tests = parts.map((p) => {
    if (/[*?]/.test(p)) {
      const re = new RegExp("^" + p.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\?/g, ".") + "$");
      return (name) => re.test(name);
    }
    const end = "." + p.replace(/^\*?\./, "");
    return (name) => name.endsWith(end);
  });
  return (name) => tests.some((f) => f(name.toLowerCase()));
}

function visible() {
  const q = $("filter").value.trim().toLowerCase();
  const all = $("showAll").checked;
  const ext = extMatcher($("exts").value);
  return links.filter(
    (l) =>
      (all || l.likely) &&
      (type === "all" || l.type === type) &&
      ext(l.name) &&
      (!q || l.url.toLowerCase().includes(q) || l.text.toLowerCase().includes(q) || l.name.toLowerCase().includes(q))
  );
}

// Seuls les liens cochés ET visibles partent (ce que montre le compteur)
function chosen() {
  return visible().filter((l) => l.checked && l.state !== "ok");
}

function updateCount(shown) {
  const n = chosen().length;
  $("count").textContent = t("pickCount", [String(n), String(shown.length)]);
  $("send").textContent = t(n === 0 ? "pickSendNone" : n === 1 ? "pickSendOne" : "pickSend", [String(n)]);
  $("send").disabled = sending || n === 0;
}

function renderTypes() {
  const box = $("types");
  box.replaceChildren();
  const all = $("showAll").checked;
  for (const k of Object.keys(TYPE_KEYS)) {
    const n = links.filter((l) => (all || l.likely) && (k === "all" || l.type === k)).length;
    const b = document.createElement("button");
    b.type = "button";
    b.className = "type" + (k === type ? " on" : "");
    b.setAttribute("role", "radio");
    b.setAttribute("aria-checked", String(k === type));
    b.dataset.type = k;
    b.textContent = t(TYPE_KEYS[k]) + " ";
    const c = document.createElement("span");
    c.className = "n";
    c.textContent = String(n);
    b.append(c);
    b.disabled = n === 0 && k !== "all";
    b.addEventListener("click", () => {
      type = k;
      render();
    });
    box.append(b);
  }
}

function render() {
  renderTypes();
  const list = $("list");
  list.replaceChildren();
  const shown = visible();
  $("empty").hidden = shown.length > 0;
  for (const l of shown) {
    const li = document.createElement("li");
    const label = document.createElement("label");
    label.title = l.url;
    const box = document.createElement("input");
    box.type = "checkbox";
    box.checked = l.checked;
    box.addEventListener("change", () => {
      l.checked = box.checked;
      updateCount(shown);
    });
    const ext = document.createElement("span");
    ext.className = "ext" + (l.ext ? " " + (l.type || "other") : " none");
    ext.textContent = l.ext || (l.url.startsWith("magnet:") ? "magnet" : "—");
    const name = document.createElement("span");
    name.className = "name";
    const b = document.createElement("b");
    b.textContent = l.name;
    const small = document.createElement("small");
    small.textContent = l.text && l.text !== l.name ? `${l.host} · ${l.text}` : l.host;
    name.append(b, small);
    const state = document.createElement("span");
    state.className = "state" + (l.state === "ok" ? " ok" : l.state ? " err" : "");
    state.textContent = l.state === "ok" ? "✓" : l.state ? "✕" : "";
    if (l.state && l.state !== "ok") state.title = l.state;
    label.append(box, ext, name, state);
    li.append(label);
    list.append(li);
  }
  updateCount(shown);
}

function setAll(on) {
  for (const l of visible()) l.checked = on;
  render();
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function sendOne(l) {
  // L'appli limite à 30 demandes / 10 s : on attend et on réessaie si elle le signale
  for (let i = 0; i < 6; i++) {
    const res = await api.runtime.sendMessage({ type: "fluxdl-add", url: l.url, referrer });
    if (res?.state !== "rate") return res?.state ?? "error";
    await sleep(2000);
  }
  return "rate";
}

const STOP_KEYS = { down: "statusNotRunning", unpaired: "statusNotPaired", noaccess: "statusNoAccess", origin: "statusBadOrigin" };

// Appli sans /add-many (0.2.8 et avant) : un par un, liens magnet exclus (refusés par /add de
// l'extension)
async function sendEach(todo) {
  let ok = 0;
  for (const l of todo) {
    const state = l.url.startsWith("magnet:") ? "url" : await sendOne(l);
    l.state = state;
    if (state === "ok") {
      ok++;
      l.checked = false;
    }
    render();
    if (STOP_KEYS[state]) return { ok, stop: state };
    await sleep(120);
  }
  return { ok, stop: "" };
}

async function sendChecked() {
  const todo = chosen();
  if (!todo.length) return;
  sending = true;
  render();
  const msg = $("msg");
  const res = await api.runtime.sendMessage({ type: "fluxdl-add-many", urls: todo.map((l) => l.url), referrer });
  let ok = 0;
  let stop = "";
  if (res?.state === "old") {
    ({ ok, stop } = await sendEach(todo.slice(res.sent ?? 0)));
  } else {
    ok = res?.sent ?? 0;
    todo.forEach((l, i) => {
      l.state = i < ok ? "ok" : res?.state || "error";
      if (i < ok) l.checked = false;
    });
    if (res?.state !== "ok") stop = STOP_KEYS[res?.state] ? res.state : "";
  }
  sending = false;
  render();
  if (stop) {
    msg.textContent = t(STOP_KEYS[stop]);
    msg.className = "msg err";
  } else {
    msg.textContent = t("pickSent", [String(ok), String(todo.length)]);
    msg.className = "msg " + (ok === todo.length ? "ok" : "err");
  }
}

async function init() {
  localizePage();
  const s = await getSettings();
  const exts = new Set([...FILE_EXTS, ...parseList(s.extensions)]);
  const res = await api.runtime.sendMessage({ type: "fluxdl-picker", id: location.hash.slice(1) });
  const data = res?.data;
  if (!data) {
    $("empty").hidden = false;
    $("send").disabled = true;
    return;
  }
  referrer = data.page || "";
  $("page").textContent = data.title || data.page || "";
  $("page").title = data.page || "";
  links = data.links.map((l) => {
    const name = nameOf(l.url);
    const ext = l.url.startsWith("magnet:") ? "" : extOfName(name);
    const kind = typeOfExt(ext);
    const likely = l.media || l.dl || !!kind || exts.has(ext) || l.url.startsWith("magnet:");
    // Images non cochées d'avance (icônes, logos…) : bouton Images puis Tout cocher
    const checked = likely && kind !== "images";
    return { ...l, name, host: l.url.startsWith("magnet:") ? "magnet" : hostOf(l.url), ext, type: kind, likely, checked, state: "" };
  });
  if (!links.some((l) => l.likely)) $("showAll").checked = true;
  $("filter").addEventListener("input", render);
  $("exts").addEventListener("input", render);
  $("showAll").addEventListener("change", render);
  $("all").addEventListener("click", () => setAll(true));
  $("none").addEventListener("click", () => setAll(false));
  $("send").addEventListener("click", sendChecked);
  render();
}

init();
