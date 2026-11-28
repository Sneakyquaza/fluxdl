// Extension 0.2.4, bouton vidéo : Chromium Playwright (headless) + fausse appli + fausse page
// « youtube » servie sur localhost (on ne peut pas usurper youtube.com : la copie de test de
// l'extension ajoute un site « test » à sites.js et rend la permission cookies obligatoire).
// Usage : NODE_PATH=~/df/node_modules node tests/extension/video.cjs [dossier de captures]
// Sans compte réel ni contenu réel : rien ne sort de 127.0.0.1 / localhost.
"use strict";
const { chromium } = require("playwright");
const fs = require("fs");
const os = require("os");
const path = require("path");
const http = require("http");
const vm = require("vm");
const assert = require("assert");

const SRC = path.join(__dirname, "..", "..", "extension");
const CAP = process.argv[2] || fs.mkdtempSync(path.join(os.tmpdir(), "fluxdl-cap-"));
const APP_PORT = 47614;
const WEB_PORT = 18970;
const TOKEN = "00112233445566778899aabbccddeeff";
let failures = 0;
const check = (name, ok, detail) => {
  console.log(`${ok ? "ok  " : "FAIL"} ${name}${detail ? " — " + detail : ""}`);
  if (!ok) failures++;
};

// ---- 1. Liste des sites (sites.js réel, sans navigateur) ----
function hostListTests() {
  const ctx = { URL };
  vm.createContext(ctx);
  vm.runInContext(fs.readFileSync(path.join(SRC, "sites.js"), "utf8") + "\nthis.videoPageSite = videoPageSite; this.videoSiteOf = videoSiteOf; this.siteOrigins = siteOrigins; this.VIDEO_SITES = VIDEO_SITES;", ctx);
  const page = (u) => ctx.videoPageSite(u)?.id ?? null;
  const cases = [
    ["https://www.youtube.com/watch?v=dQw4w9WgXcQ", "youtube"],
    ["https://m.youtube.com/watch?v=dQw4w9WgXcQ&t=42", "youtube"],
    ["https://www.youtube.com/live/jfKfPfyJRdk", "youtube"],
    ["https://www.youtube.com/shorts/abcdefghijk", "youtube"],
    ["https://youtu.be/dQw4w9WgXcQ", "youtube"],
    ["https://www.youtube.com/", null],
    ["https://www.youtube.com/feed/subscriptions", null],
    ["https://www.youtube.com/watch", null],
    ["https://www.youtube.com/@chaine", "youtube"],
    ["https://notyoutube.com/watch?v=dQw4w9WgXcQ", null],
    ["https://youtube.com.evil.example/watch?v=dQw4w9WgXcQ", null],
    ["https://www.twitch.tv/videos/2123456789", "twitch"],
    ["https://www.twitch.tv/videos/2123456789?t=1h2m3s", "twitch"],
    ["https://m.twitch.tv/videos/2123456789", "twitch"],
    ["https://www.twitch.tv/somechannel/v/2123456789", "twitch"],
    ["https://www.twitch.tv/somechannel/video/2123456789", "twitch"],
    ["https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG&index=2", "youtube"],
    ["https://www.youtube.com/watch?app=desktop&v=dQw4w9WgXcQ&t=1m2s", "youtube"],
    ["https://music.youtube.com/watch?v=dQw4w9WgXcQ", "youtube"],
    ["https://youtu.be/dQw4w9WgXcQ?si=abc&t=42", "youtube"],
    ["https://www.youtube.com/shorts/abcdefghijk?feature=share", "youtube"],
    ["https://www.youtube.com/live/jfKfPfyJRdk?si=x", "youtube"],
    ["https://www.twitch.tv/somechannel/clip/FunnyClipName-AbC123", "twitch"],
    ["https://clips.twitch.tv/FunnyClipName-AbC123", "twitch"],
    ["https://www.twitch.tv/somechannel", "twitch"],
    ["https://www.twitch.tv/directory", null],
    ["https://www.twitch.tv/settings", null],
    ["https://www.twitch.tv/somechannel/videos", "twitch"],
    ["https://www.twitch.tv/somechannel/videos?filter=archives&sort=time", "twitch"],
    ["https://www.twitch.tv/videos/videos", null],
    ["https://www.twitch.tv/directory/videos", null],
    ["https://www.youtube.com/playlist?list=PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG", "youtube"],
    ["https://www.youtube.com/playlist", null],
    ["https://www.youtube.com/@chaine/videos", "youtube"],
    ["https://www.youtube.com/@chaine/streams/", "youtube"],
    ["https://www.youtube.com/@chaine/community", null],
    ["https://www.youtube.com/@chaine/playlists", null],
    ["https://www.youtube.com/channel/UCvjgXvBlbQiydffZU7m1_aw", "youtube"],
    ["https://www.youtube.com/c/TheCodingTrain/videos", "youtube"],
    ["https://www.youtube.com/channel/", null],
    ["https://youtu.be/dQw4w9WgXcQ?list=PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG", "youtube"],
    ["https://www.twitch.tv/", null],
    ["ftp://www.youtube.com/watch?v=dQw4w9WgXcQ", null],
    ["pas une url", null],
  ];
  for (const [u, want] of cases) check(`page vidéo ${u}`, page(u) === want, `→ ${page(u)}`);
  // Le bouton envoie location.href et l'arrière-plan exige le même site que l'onglet
  for (const [u, want] of cases) if (want) check(`même site ${u}`, ctx.videoSiteOf(u) === ctx.videoPageSite(u));
  check("site sans être une page vidéo", ctx.videoSiteOf("https://www.youtube.com/")?.id === "youtube");
  const yt = ctx.VIDEO_SITES.find((s) => s.id === "youtube");
  check("cookies YouTube : youtube.com seulement", JSON.stringify(ctx.siteOrigins(yt, yt.cookieDomains)) === JSON.stringify(["*://youtube.com/*", "*://*.youtube.com/*"]));
}

// ---- Copie de test de l'extension ----
function testBuild() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "fluxdl-ext-"));
  fs.cpSync(SRC, dir, { recursive: true, filter: (p) => !p.includes(`${path.sep}dist`) });
  fs.appendFileSync(
    path.join(dir, "sites.js"),
    `\n// Construction de test seulement\nVIDEO_SITES.push({ id: "test", name: "Test (localhost)", hosts: ["localhost"],
  page: (u) => u.pathname === "/watch" && !!u.searchParams.get("v"), cookieDomains: ["localhost"], player: ["#movie_player", "video"] });\n`
  );
  const m = JSON.parse(fs.readFileSync(path.join(dir, "manifest.json"), "utf8"));
  m.permissions.push("cookies"); // pas de dialogue de permission en headless
  m.host_permissions.push("*://localhost/*", "*://*.localhost/*");
  fs.writeFileSync(path.join(dir, "manifest.json"), JSON.stringify(m, null, 2));
  // Ombre ouverte : le test lit le libellé du bouton
  const cs = path.join(dir, "content.js");
  fs.writeFileSync(cs, fs.readFileSync(cs, "utf8").replaceAll('mode: "closed"', 'mode: "open"'));
  return dir;
}

// ---- Fausse appli : enregistre les /add ----
const received = [];
let addCode = 200; // réponse forcée de /add (503 timeout, 400 appli ancienne, 409…)
let pairs = 0;
function fakeApp() {
  return http
    .createServer((req, res) => {
      const cors = { "Access-Control-Allow-Origin": req.headers.origin || "*", "Access-Control-Allow-Headers": "content-type,x-fluxdl-token" };
      if (req.method === "OPTIONS") return res.writeHead(204, cors).end();
      // Appairage accordé tout de suite (« Autoriser » cliqué)
      if (req.url === "/pair") {
        pairs++;
        return res.writeHead(200, cors).end(JSON.stringify({ token: TOKEN }));
      }
      if (req.headers["x-fluxdl-token"] !== TOKEN) return res.writeHead(401, cors).end("{}");
      let body = "";
      req.on("data", (c) => (body += c));
      req.on("end", () => {
        if (req.url === "/ping") return res.writeHead(200, cors).end(JSON.stringify({ app: "fluxdl", version: "0.2.4" }));
        if (req.url === "/add") {
          if (addCode !== 200) return res.writeHead(addCode, cors).end(JSON.stringify({ ok: false, status: addCode === 503 ? "timeout" : "refused" }));
          received.push(JSON.parse(body));
          return res.writeHead(200, cors).end(JSON.stringify({ ok: true, status: "added" }));
        }
        res.writeHead(404, cors).end("{}");
      });
    })
    .listen(APP_PORT, "127.0.0.1");
}

// ---- Fausse page « youtube.com/watch » (structure : #movie_player > video en blob:) ----
const PAGE = (title) => `<!doctype html><meta charset="utf-8"><title>${title}</title>
<style>body{margin:0;font:14px sans-serif;background:#0f0f0f;color:#fff} header{height:56px;background:#202020}
#columns{display:flex;gap:24px;padding:24px} #movie_player{width:854px;height:480px;background:#000;position:relative}
#movie_player video{width:100%;height:100%} #movie_player .ttl{position:absolute;bottom:56px;left:16px;color:#ccc}
#secondary{width:360px} .card{height:90px;background:#272727;margin-bottom:8px;border-radius:8px}</style>
<header></header><div id="columns"><div><div id="movie_player"><video></video><div class="ttl">lecteur simulé (blob:)</div></div>
<h2 id="t">${title}</h2></div><div id="secondary"><div class="card"></div><div class="card"></div><div class="card"></div></div></div>
<script>document.querySelector("video").src = URL.createObjectURL(new Blob(["x"], { type: "video/mp4" }));</script>`;
function fakeSite() {
  return http
    .createServer((req, res) => {
      res.writeHead(200, { "Content-Type": "text/html; charset=utf-8", "Set-Cookie": "SID=secret-de-test; Path=/; HttpOnly" });
      res.end(PAGE(req.url.startsWith("/watch") ? "Vidéo de test" : "Accueil"));
    })
    .listen(WEB_PORT, "127.0.0.1");
}

(async () => {
  hostListTests();
  const ext = testBuild();
  const app = fakeApp();
  const site = fakeSite();
  const ctx = await chromium.launchPersistentContext(fs.mkdtempSync(path.join(os.tmpdir(), "fluxdl-profil-")), {
    channel: "chromium",
    headless: true,
    viewport: { width: 1320, height: 760 },
    locale: "fr-FR",
    args: [`--disable-extensions-except=${ext}`, `--load-extension=${ext}`, "--lang=fr"],
  });
  let [sw] = ctx.serviceWorkers();
  if (!sw) sw = await ctx.waitForEvent("serviceworker");
  const extId = new URL(sw.url()).host;
  await sw.evaluate(([t, p]) => chrome.storage.local.set({ token: t, port: p, floatButton: true }), [TOKEN, APP_PORT]);
  // Cookie d'un autre site : ne doit jamais partir
  await ctx.addCookies([{ name: "OTHER", value: "x", url: "http://example.org/" }, { name: "OTHER", value: "y", url: "http://127.0.0.1/" }]);
  check("cookies d'autres sites présents dans le navigateur", (await ctx.cookies()).filter((c) => c.name === "OTHER").length === 2);

  const page = await ctx.newPage();
  const base = `http://localhost:${WEB_PORT}`;
  await page.goto(`${base}/watch?v=abcdefgh`);
  const host = page.locator("fluxdl-video");
  await host.waitFor({ state: "visible", timeout: 5000 });
  const box = await host.boundingBox();
  const player = await page.locator("#movie_player").boundingBox();
  check("bouton visible dans le coin haut droit du lecteur", box && box.x > player.x + player.width - 160 && box.x + box.width <= player.x + player.width && Math.abs(box.y - player.y - 12) < 2, JSON.stringify(box));
  await page.screenshot({ path: path.join(CAP, "ext_video_button.png") });

  // Un clic = un envoi (le bouton revient à l'état normal 2,5 s après)
  const clickBtn = async () => {
    const n = received.length;
    const b = await host.boundingBox();
    await page.mouse.click(b.x + b.width / 2, b.y + b.height / 2);
    await page.waitForTimeout(700);
    check("  (le clic a envoyé une demande)", received.length === n + 1);
    await page.waitForTimeout(2000);
  };
  // Clic simulé par la page : ignoré
  await page.evaluate(() => document.querySelector("fluxdl-video").dispatchEvent(new MouseEvent("click", { bubbles: true })));
  await page.waitForTimeout(300);
  check("clic simulé par la page ignoré", received.length === 0);

  {
    const b = await host.boundingBox();
    await page.mouse.click(b.x + b.width / 2, b.y + b.height / 2);
    await page.waitForTimeout(500);
    await page.screenshot({ path: path.join(CAP, "ext_video_button_sent.png"), clip: { x: player.x + player.width - 300, y: player.y, width: 300, height: 80 } });
    await page.waitForTimeout(2300);
  }
  let last = received.at(-1);
  check("envoi de la page (tâche vidéo, sans cookies)", received.length === 1 && last.url === `${base}/watch?v=abcdefgh` && last.kind === "video" && !("video_cookies" in last), JSON.stringify(last));

  // Navigation sans rechargement (SPA) : hors page vidéo → caché ; nouvelle vidéo → visible
  await page.evaluate(() => history.pushState({}, "", "/feed/subscriptions"));
  await page.waitForTimeout(1300);
  check("SPA : caché hors page vidéo", !(await host.isVisible()));
  await page.evaluate(() => {
    history.pushState({}, "", "/watch?v=zyxwvuts");
    document.dispatchEvent(new Event("yt-navigate-finish"));
  });
  await page.waitForTimeout(200);
  check("SPA : yt-navigate-finish → visible tout de suite", await host.isVisible());
  await clickBtn();
  last = received.at(-1);
  check("SPA : nouvelle adresse envoyée", last.url === `${base}/watch?v=zyxwvuts`, last.url);

  // Options : interrupteur « cookies » par site
  const opt = await ctx.newPage();
  await opt.setViewportSize({ width: 760, height: 1000 });
  await opt.goto(`chrome-extension://${extId}/options.html`);
  const toggle = opt.locator("#videoCookies-test");
  check("cookies désactivés par défaut", !(await toggle.isChecked()) && !(await opt.locator("#videoCookies-youtube").isChecked()));
  await opt.locator('label:has(#videoCookies-test) .track').click();
  await opt.waitForTimeout(400);
  check("interrupteur activé", await toggle.isChecked());
  const card = opt.locator("section", { has: opt.locator("#videoSites") });
  await card.scrollIntoViewIfNeeded();
  await card.screenshot({ path: path.join(CAP, "ext_options_video_cookies.png") });

  await page.bringToFront();
  await page.waitForTimeout(300);
  await clickBtn();
  last = received.at(-1);
  const jar = last.video_cookies || [];
  const sid = jar.find((c) => c.name === "SID");
  check("toggle activé : cookies du site envoyés", !!sid && sid.value === "secret-de-test" && sid.domain === "localhost" && sid.path === "/" && sid.httpOnly === true && sid.secure === false && sid.expiry === 0, JSON.stringify(jar.map((c) => ({ ...c, value: "***" }))));
  check("toggle activé : aucun cookie d'un autre site", !jar.some((c) => c.name === "OTHER" || !/localhost$/.test(c.domain)));

  await opt.bringToFront();
  await opt.locator('label:has(#videoCookies-test) .track').click();
  await opt.waitForTimeout(400);
  await page.bringToFront();
  await clickBtn();
  check("toggle désactivé : plus de cookies", !("video_cookies" in received.at(-1)));

  // Menu contextuel : même chemin (sendVideo) ; une page qui n'est pas une vidéo → rien
  const n = received.length;
  const st = await sw.evaluate((u) => (videoPageSite(u) ? sendVideo(u, videoPageSite(u), {}) : "not-video"), `${base}/feed`);
  check("menu : page non vidéo refusée", st === "not-video" && received.length === n);
  const st2 = await sw.evaluate((u) => sendVideo(u, videoPageSite(u), {}), `${base}/watch?v=menutest`);
  check("menu : page vidéo envoyée", st2.state === "ok" && received.at(-1).url.endsWith("menutest"));

  // Échec : raison courte sur le bouton (diagnostic depuis une capture)
  const clickLabel = async (shot) => {
    const b = await host.boundingBox();
    await page.mouse.click(b.x + b.width / 2, b.y + b.height / 2);
    await page.waitForTimeout(700);
    const text = await page.locator("fluxdl-video button").textContent();
    if (shot) await page.screenshot({ path: path.join(CAP, shot), clip: { x: player.x + player.width - 300, y: player.y, width: 300, height: 80 } });
    await page.waitForTimeout(2000);
    return text;
  };
  for (const [code, want] of [[503, "Appli occupée"], [400, "Appli à mettre à jour"], [404, "Appli à mettre à jour"], [409, "Refusé (409)"], [403, "Refusé (403)"]]) {
    addCode = code;
    const text = await clickLabel(code === 400 ? "ext_video_button_update.png" : null);
    check(`libellé ${code} : ${want}`, text.endsWith(want), text);
  }
  addCode = 200;
  // Pas de jeton (extension réinstallée) : appairage automatique puis envoi
  await sw.evaluate(() => chrome.storage.local.set({ token: "" }));
  const before = received.length;
  const pairsBefore = pairs; // l'installation en a déjà fait une
  const text = await clickLabel();
  const tok = await sw.evaluate(() => chrome.storage.local.get("token"));
  check("sans jeton : appairage auto puis envoi", pairs === pairsBefore + 1 && tok.token === TOKEN && received.length === before + 1 && text.endsWith("Envoyé"), `${pairs} ${text}`);
  // Appli fermée : libellé dédié
  app.close();
  await page.waitForTimeout(200);
  const down = await clickLabel();
  check("appli fermée : « fluxdl fermé »", down.endsWith("fluxdl fermé"), down);

  // Interrupteur « bouton » coupé : plus de bouton vidéo
  await sw.evaluate(() => chrome.storage.local.set({ floatButton: false }));
  await page.reload();
  await page.waitForTimeout(1500);
  check("bouton coupé dans les options : absent", !(await host.isVisible()));

  await ctx.close();
  site.close();
  console.log(failures ? `${failures} échec(s)` : "tout est vert");
  console.log(`captures : ${CAP}`);
  process.exit(failures ? 1 : 0);
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
