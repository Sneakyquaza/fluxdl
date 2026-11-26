// Script de contenu : bouton flottant « ⬇ fluxdl » au survol des liens de fichiers (vérifiés par
// l'arrière-plan : pas de bouton sur une page web) et des
// vidéos/sons, bouton près du lecteur sur les pages de vidéo des sites de sites.js (chargé
// avant ce script), et collecte des liens pour la page « tout télécharger ».
// Ne lit jamais le stockage (le jeton y est) et ne parle qu'à l'arrière-plan.
"use strict";

(() => {
  const api = globalThis.browser ?? chrome;
  if (globalThis.__fluxdlContent) return;
  globalThis.__fluxdlContent = true;

  const HIDE_MS = 700;
  let config = { floatButton: false, exts: [] };
  let host = null; // élément hôte dans la page
  let btn = null; // bouton dans l'ombre fermée
  let current = null; // { el, url }
  let pointed = null; // lien survolé en attente de la sonde
  let hideTimer = 0;
  let resetTimer = 0;

  const t = (k) => api.i18n.getMessage(k) || k;

  function send(msg) {
    try {
      return api.runtime.sendMessage(msg).catch(() => null);
    } catch {
      return Promise.resolve(null); // extension rechargée : contexte invalide
    }
  }

  async function loadConfig() {
    const c = await send({ type: "fluxdl-config" });
    if (c && Array.isArray(c.exts)) config = { floatButton: !!c.floatButton, exts: c.exts };
    if (!config.floatButton) hide();
    checkVideoPage();
  }

  function isHttp(u) {
    return /^https?:\/\//i.test(u || "");
  }

  function extOf(u) {
    try {
      const name = new URL(u).pathname.split("/").pop();
      const dot = name.lastIndexOf(".");
      return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
    } catch {
      return "";
    }
  }

  function likelyFile(u) {
    const e = extOf(u);
    return !!e && config.exts.includes(e);
  }

  function mediaSrc(el) {
    let src = el.currentSrc || el.src || "";
    if (!isHttp(src)) {
      const s = el.querySelector("source[src]");
      src = s ? s.src : "";
    }
    return isHttp(src) ? src : ""; // blob:/data: (flux MSE) : impossible à télécharger tel quel
  }

  // Lien de fichier sous le pointeur
  function linkTarget(node) {
    const a = node?.closest?.("a[href]");
    if (!a || !isHttp(a.href)) return null;
    if (a.hasAttribute("download") || likelyFile(a.href)) return { el: a, url: a.href, media: false };
    return null;
  }

  // Vidéo/son sous le pointeur (souvent recouvert par les contrôles du lecteur)
  function mediaAt(x, y) {
    for (const el of document.querySelectorAll("video, audio")) {
      const r = el.getBoundingClientRect();
      if (r.width < 40 || r.height < 20) continue;
      if (x >= r.left && x <= r.right && y >= r.top && y <= r.bottom) {
        const url = mediaSrc(el);
        if (url) return { el, url, media: true };
      }
    }
    return null;
  }

  const CSS = `
    :host { all: initial; }
    button {
      all: unset;
      box-sizing: border-box;
      display: inline-flex; align-items: center; gap: 5px;
      height: 24px; padding: 0 11px 0 9px;
      font: 600 12px/1 "Segoe UI", "Open Sans", system-ui, sans-serif;
      color: #fff; letter-spacing: .2px; white-space: nowrap;
      text-shadow: 0 -1px 0 rgba(0, 40, 90, .45);
      cursor: pointer; border-radius: 999px;
      border: 1px solid #0f56a8;
      background:
        linear-gradient(180deg, rgba(255,255,255,.7) 0%, rgba(255,255,255,.18) 50%, rgba(255,255,255,0) 51%),
        radial-gradient(ellipse at 50% 120%, rgba(120,240,255,.9), rgba(120,240,255,0) 60%),
        linear-gradient(180deg, #5ab2f5, #1e78d6 60%, #2a8ae8);
      box-shadow: inset 0 0 0 1px rgba(255,255,255,.45), 0 2px 6px rgba(0,40,90,.45);
      opacity: 0; transform: translateY(2px) scale(.96);
      transition: opacity .12s ease-out, transform .12s ease-out, filter .1s;
    }
    button.on { opacity: 1; transform: none; }
    button:hover { filter: brightness(1.1) saturate(1.1); }
    button:active { filter: brightness(.92); }
    button:focus-visible { outline: 2px solid #fff; outline-offset: 1px; box-shadow: 0 0 0 4px #1e78d6; }
    button.ok {
      border-color: #1d7d27;
      background:
        linear-gradient(180deg, rgba(255,255,255,.65) 0%, rgba(255,255,255,.15) 50%, rgba(255,255,255,0) 51%),
        linear-gradient(180deg, #6cd977, #2fae3c 60%, #3cbf48);
    }
    button.err {
      border-color: #9c2219;
      background:
        linear-gradient(180deg, rgba(255,255,255,.6) 0%, rgba(255,255,255,.12) 50%, rgba(255,255,255,0) 51%),
        linear-gradient(180deg, #f07a70, #d63a2f 60%, #e0483d);
    }
    .ar { font-size: 13px; }
    button.big { height: 30px; padding: 0 14px 0 11px; font-size: 13px; gap: 6px; }
    button.big .ar { font-size: 15px; }
    @media (prefers-reduced-motion: reduce) { button { transition: none; transform: none; } }
  `;

  const HOST_CSS =
    "all:initial;position:absolute;z-index:2147483647;top:0;left:0;display:none;pointer-events:auto;";

  // Contre le détournement de clic : le bouton doit être vraiment visible (la page peut
  // modifier le style de l'hôte, ou rendre la page entière presque transparente) et être
  // l'élément sous le pointeur.
  function reallyVisible(e, h = host) {
    if (document.elementFromPoint(e.clientX, e.clientY) !== h) return false;
    for (let el = h; el; el = el.parentElement) {
      const cs = getComputedStyle(el);
      if (Number(cs.opacity) < 0.9 || cs.visibility !== "visible" || (cs.filter && cs.filter !== "none")) return false;
    }
    return true;
  }

  function ensureHost() {
    if (host?.isConnected) return;
    host = document.createElement("fluxdl-float");
    // Styles en ligne sur l'hôte : le CSS du site ne peut pas l'atteindre dans l'ombre,
    // et l'hôte hors flux (absolute) ne décale rien.
    host.style.cssText = HOST_CSS;
    const root = host.attachShadow({ mode: "closed" });
    const style = document.createElement("style");
    style.textContent = CSS;
    btn = document.createElement("button");
    btn.type = "button";
    root.append(style, btn);
    btn.addEventListener("click", onClick);
    host.addEventListener("mouseenter", () => clearTimeout(hideTimer));
    host.addEventListener("mouseleave", scheduleHide);
    document.documentElement.appendChild(host);
  }

  function setLabel(icon, text, b = btn) {
    b.replaceChildren();
    const ar = document.createElement("span");
    ar.className = "ar";
    ar.setAttribute("aria-hidden", "true");
    ar.textContent = icon;
    b.append(ar, document.createTextNode(text));
  }

  function show(target) {
    clearTimeout(hideTimer);
    if (current && current.el === target.el && current.url === target.url && host?.style.display !== "none") return;
    ensureHost();
    // Style de l'hôte réappliqué à chaque affichage (la page a pu le changer).
    host.style.cssText = HOST_CSS;
    current = target;
    clearTimeout(resetTimer);
    btn.className = "";
    btn.disabled = false;
    setLabel("⬇", "fluxdl");
    let name = "";
    try {
      name = decodeURIComponent(new URL(target.url).pathname.split("/").pop() || "");
    } catch {
      // nom illisible
    }
    btn.setAttribute("aria-label", t("floatLabel") + (name ? " : " + name : ""));
    btn.title = t("floatLabel");
    place();
    host.style.display = "block";
    requestAnimationFrame(() => btn.classList.add("on"));
  }

  // Lien : juste à droite du lien (ou au-dessus s'il touche le bord) ; média : coin haut droit
  function place() {
    if (!current) return;
    const r = current.el.getBoundingClientRect();
    const sx = window.scrollX;
    const sy = window.scrollY;
    const w = 86;
    const h = 24;
    const vw = document.documentElement.clientWidth;
    let x;
    let y;
    if (current.media) {
      x = r.right - w - 8;
      y = r.top + 8;
    } else {
      x = r.right + 6;
      y = r.top + (r.height - h) / 2;
      if (x + w > vw - 4) {
        x = Math.max(4, Math.min(r.left, vw - w - 4));
        y = r.top - h - 4;
      }
    }
    host.style.left = Math.round(x + sx) + "px";
    host.style.top = Math.round(Math.max(0, y + sy)) + "px";
  }

  function hide() {
    clearTimeout(hideTimer);
    if (host) host.style.display = "none";
    btn?.classList.remove("on");
    current = null;
  }

  function scheduleHide() {
    clearTimeout(hideTimer);
    hideTimer = setTimeout(hide, HIDE_MS);
  }

  async function onClick(e) {
    e.preventDefault();
    e.stopPropagation();
    if (!e.isTrusted || !current || btn.disabled) return; // clic simulé par la page : ignoré
    if (!reallyVisible(e)) return;
    const url = current.url;
    btn.disabled = true;
    setLabel("…", "fluxdl");
    const res = await send({ type: "fluxdl-add", url, title: document.title });
    const state = res?.state;
    if (state === "ok" || state === "browser") {
      btn.className = "on ok";
      setLabel("✓", t(state === "ok" ? "floatSent" : "floatBrowser"));
    } else {
      btn.className = "on err";
      setLabel("✕", t("floatError"));
    }
    clearTimeout(resetTimer);
    resetTimer = setTimeout(hide, 1600);
  }

  // Lien : le bouton n'apparaît que si l'arrière-plan confirme que c'est un fichier (pas une
  // page web) ; média : tout de suite (adresse réelle d'un <video>/<audio>).
  function offer(target) {
    if (target.media || (current && current.el === target.el && current.url === target.url)) {
      show(target);
      return;
    }
    if (pointed && pointed.el === target.el && pointed.url === target.url) return;
    pointed = target;
    send({ type: "fluxdl-probe", url: target.url }).then((res) => {
      if (res?.ok !== true || pointed !== target || !config.floatButton) return;
      pointed = null;
      show(target);
    });
  }

  let pending = null;
  function onPointer(e) {
    if (!config.floatButton) return;
    if (e.target === host) {
      clearTimeout(hideTimer);
      return;
    }
    pending = e;
    if (onPointer.raf) return;
    onPointer.raf = requestAnimationFrame(() => {
      onPointer.raf = 0;
      const ev = pending;
      const target = linkTarget(ev.target) || mediaAt(ev.clientX, ev.clientY);
      if (target) offer(target);
      else {
        pointed = null;
        if (current) scheduleHide();
      }
    });
  }

  function onFocus(e) {
    if (!config.floatButton || e.target === host) return;
    const target = linkTarget(e.target);
    if (target) offer(target);
  }

  // Liens pour la page « tout télécharger »
  function collect() {
    const links = [];
    for (const a of document.querySelectorAll("a[href]")) {
      if (!isHttp(a.href)) continue;
      const text = (a.textContent || a.title || "").trim().replace(/\s+/g, " ").slice(0, 200);
      links.push({ url: a.href, text, dl: a.hasAttribute("download") });
      if (links.length >= 3000) break;
    }
    for (const el of document.querySelectorAll("video, audio")) {
      const url = mediaSrc(el);
      if (url) links.push({ url, text: el.title || "", media: true });
    }
    return { links, page: location.href, title: document.title };
  }

  // ---- Bouton vidéo : pages de vidéo des sites de sites.js (lecteur en blob:) ----
  // Toujours visible près du lecteur (coin haut droit), sinon en bas à droite de la fenêtre ;
  // envoie le lien de la PAGE. Navigation sans rechargement (YouTube, Twitch) : événement
  // yt-navigate-finish, popstate et adresse relue chaque seconde (sites vidéo seulement).
  const VIDEO_HOST_CSS =
    "all:initial;position:fixed;z-index:2147483646;top:0;left:0;display:none;pointer-events:auto;";
  let vHost = null;
  let vBtn = null;
  let vSite = null;
  let vUrl = "";
  let vReset = 0;

  function videoIdle() {
    vBtn.className = "big on";
    vBtn.disabled = false;
    setLabel("⬇", "fluxdl", vBtn);
  }

  function ensureVideoHost() {
    if (vHost?.isConnected) return;
    vHost = document.createElement("fluxdl-video");
    vHost.style.cssText = VIDEO_HOST_CSS;
    const root = vHost.attachShadow({ mode: "closed" });
    const style = document.createElement("style");
    style.textContent = CSS;
    vBtn = document.createElement("button");
    vBtn.type = "button";
    vBtn.title = t("videoButtonLabel");
    vBtn.setAttribute("aria-label", t("videoButtonLabel"));
    root.append(style, vBtn);
    vBtn.addEventListener("click", onVideoClick);
    document.documentElement.appendChild(vHost);
    videoIdle();
  }

  function findPlayer(site) {
    for (const sel of site.player) {
      for (const el of document.querySelectorAll(sel)) {
        const r = el.getBoundingClientRect();
        if (r.width >= 200 && r.height >= 110) return r;
      }
    }
    return null;
  }

  function placeVideo() {
    if (!vHost || !vSite) return;
    const r = findPlayer(vSite);
    const vw = document.documentElement.clientWidth;
    const vh = window.innerHeight;
    const w = vBtn.offsetWidth || 96;
    const h = 30;
    let x = vw - w - 20;
    let y = vh - h - 20;
    // Lecteur assez visible : dans son coin haut droit
    if (r && r.top >= -40 && r.top + 12 + h < vh && r.right - w - 12 > 0) {
      x = Math.min(r.right - w - 12, vw - w - 4);
      y = Math.max(4, r.top + 12);
    }
    vHost.style.left = Math.round(x) + "px";
    vHost.style.top = Math.round(y) + "px";
  }

  function checkVideoPage() {
    const site = config.floatButton ? videoPageSite(location.href) : null;
    if (!site) {
      if (vHost) vHost.style.display = "none";
      vSite = null;
      vUrl = "";
      return;
    }
    ensureVideoHost();
    // Style réappliqué (la page a pu le changer), nouvelle vidéo = bouton remis à neuf
    vHost.style.cssText = VIDEO_HOST_CSS;
    if (vUrl !== location.href) {
      clearTimeout(vReset);
      videoIdle();
    }
    vSite = site;
    vUrl = location.href;
    vHost.style.display = "block";
    placeVideo();
  }

  async function onVideoClick(e) {
    e.preventDefault();
    e.stopPropagation();
    if (!e.isTrusted || !vSite || vBtn.disabled) return; // clic simulé par la page : ignoré
    if (!reallyVisible(e, vHost)) return;
    vBtn.disabled = true;
    setLabel("…", "fluxdl", vBtn);
    const res = await send({ type: "fluxdl-video", url: location.href, title: document.title });
    const state = res?.state;
    if (state === "ok") {
      vBtn.className = "big on ok";
      setLabel("✓", t("floatSent"), vBtn);
    } else {
      vBtn.className = "big on err";
      setLabel("✕", t(state === "down" ? "floatNotRunning" : "floatError"), vBtn);
    }
    clearTimeout(vReset);
    vReset = setTimeout(videoIdle, 2500);
  }

  if (videoSiteOf(location.href)) {
    document.addEventListener("yt-navigate-finish", checkVideoPage);
    window.addEventListener("popstate", checkVideoPage);
    window.addEventListener("resize", placeVideo, { passive: true });
    window.addEventListener("scroll", placeVideo, { passive: true });
    setInterval(checkVideoPage, 1000);
  }

  api.runtime.onMessage.addListener((msg, sender, sendResponse) => {
    if (sender.id !== api.runtime.id || msg?.type !== "fluxdl-collect") return;
    sendResponse(collect());
  });

  document.addEventListener("mouseover", onPointer, { capture: true, passive: true });
  document.addEventListener("mousemove", (e) => current?.media && onPointer(e), { capture: true, passive: true });
  document.addEventListener("focusin", onFocus, true);
  document.addEventListener("focusout", (e) => current && e.relatedTarget !== host && scheduleHide(), true);
  window.addEventListener("scroll", () => current && place(), { passive: true });
  window.addEventListener("resize", () => current && place(), { passive: true });
  // Réglage relu quand on revient sur l'onglet (pas d'accès direct au stockage)
  document.addEventListener("visibilitychange", () => document.visibilityState === "visible" && loadConfig());
  loadConfig();
})();
