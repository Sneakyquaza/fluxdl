// Page « tout télécharger » : liste des liens de la page, cases à cocher, envoi un par un.
"use strict";

const $ = (id) => document.getElementById(id);
let links = []; // { url, text, media, ext, likely, checked, state }
let referrer = "";
let sending = false;

function extOfUrl(u) {
  try {
    const name = new URL(u).pathname.split("/").pop();
    const dot = name.lastIndexOf(".");
    return dot > 0 ? name.slice(dot + 1).toLowerCase().slice(0, 8) : "";
  } catch {
    return "";
  }
}

function nameOf(u) {
  try {
    const x = new URL(u);
    return decodeURIComponent(x.pathname.split("/").pop() || "") || x.hostname;
  } catch {
    return u;
  }
}

function visible() {
  const q = $("filter").value.trim().toLowerCase();
  const all = $("showAll").checked;
  return links.filter((l) => (all || l.likely) && (!q || l.url.toLowerCase().includes(q) || l.text.toLowerCase().includes(q)));
}

function updateCount() {
  const n = links.filter((l) => l.checked).length;
  $("count").textContent = t("pickCount", [String(n)]);
  $("send").disabled = sending || n === 0;
}

function render() {
  const list = $("list");
  list.replaceChildren();
  const shown = visible();
  $("empty").hidden = shown.length > 0;
  for (const l of shown) {
    const li = document.createElement("li");
    const label = document.createElement("label");
    const box = document.createElement("input");
    box.type = "checkbox";
    box.checked = l.checked;
    box.addEventListener("change", () => {
      l.checked = box.checked;
      updateCount();
    });
    const ext = document.createElement("span");
    ext.className = "ext" + (l.ext ? "" : " none");
    ext.textContent = l.ext || (l.media ? "media" : "—");
    const name = document.createElement("span");
    name.className = "name";
    const b = document.createElement("b");
    b.textContent = l.text || nameOf(l.url);
    const small = document.createElement("small");
    small.textContent = l.url;
    small.title = l.url;
    name.append(b, small);
    const state = document.createElement("span");
    state.className = "state" + (l.state === "ok" ? " ok" : l.state ? " err" : "");
    state.textContent = l.state === "ok" ? "✓" : l.state ? "✕" : "";
    if (l.state && l.state !== "ok") state.title = l.state;
    label.append(box, ext, name, state);
    li.append(label);
    list.append(li);
  }
  updateCount();
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

async function sendChecked() {
  const todo = links.filter((l) => l.checked && l.state !== "ok");
  if (!todo.length) return;
  sending = true;
  updateCount();
  let ok = 0;
  let stop = "";
  for (const l of todo) {
    const state = await sendOne(l);
    l.state = state;
    if (state === "ok") {
      ok++;
      l.checked = false;
    }
    render();
    if (["down", "unpaired", "noaccess", "origin"].includes(state)) {
      stop = state;
      break;
    }
    await sleep(120);
  }
  sending = false;
  const msg = $("msg");
  if (stop) {
    const keys = { down: "statusNotRunning", unpaired: "statusNotPaired", noaccess: "statusNoAccess", origin: "statusBadOrigin" };
    msg.textContent = t(keys[stop]);
    msg.className = "msg err";
  } else {
    msg.textContent = t("pickSent", [String(ok), String(todo.length)]);
    msg.className = "msg " + (ok === todo.length ? "ok" : "err");
  }
  updateCount();
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
    const ext = extOfUrl(l.url);
    const likely = l.media || l.dl || exts.has(ext);
    return { ...l, ext, likely, checked: likely, state: "" };
  });
  if (!links.some((l) => l.likely)) $("showAll").checked = true;
  $("filter").addEventListener("input", render);
  $("showAll").addEventListener("change", render);
  $("all").addEventListener("click", () => setAll(true));
  $("none").addEventListener("click", () => setAll(false));
  $("send").addEventListener("click", sendChecked);
  render();
}

init();
