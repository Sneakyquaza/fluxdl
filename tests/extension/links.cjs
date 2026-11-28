// 0.2.9 lot F, liens d'une page : collecte par le script de contenu (page locale de ~30 liens
// variés), sélection, page de choix (filtres type / extension / motif / texte, cases,
// compteur), envoi groupé /add-many à une fausse appli (référent + cookies du même site
// seulement), repli un par un pour une appli sans /add-many, bouton du popup.
// Usage : NODE_PATH=~/df/node_modules node tests/extension/links.cjs [dossier de captures]
// Rien ne sort de 127.0.0.1 / localhost.
"use strict";
const { chromium } = require("playwright");
const fs = require("fs");
const os = require("os");
const path = require("path");
const http = require("http");
const vm = require("vm");

const SRC = path.join(__dirname, "..", "..", "extension");
const CAP = process.argv[2] || fs.mkdtempSync(path.join(os.tmpdir(), "fluxdl-cap-"));
const APP_PORT = 47615;
const WEB_PORT = 18972;
const TOKEN = "00112233445566778899aabbccddeeff";
let failures = 0;
const check = (name, ok, detail) => {
  console.log(`${ok ? "ok  " : "FAIL"} ${name}${detail ? " — " + detail : ""}`);
  if (!ok) failures++;
};

// ---- Table des types : la même que l'appli (src/apres.rs) ----
function typeTableTests() {
  const rs = fs.readFileSync(path.join(__dirname, "..", "..", "src", "apres.rs"), "utf8");
  const block = rs.slice(rs.indexOf("const EXTENSIONS"), rs.indexOf("impl Categorie"));
  const app = {};
  for (const m of block.matchAll(/Categorie::(\w+),\s*&\[([^\]]*)\]/g)) app[m[1]] = [...m[2].matchAll(/"(\w+)"/g)].map((x) => x[1]).join(",");
  const ctx = {};
  vm.createContext(ctx);
  const js = fs.readFileSync(path.join(SRC, "picker.js"), "utf8");
  vm.runInContext(js.slice(js.indexOf("const TYPES"), js.indexOf("const TYPE_KEYS")) + "\nthis.TYPES = TYPES;", ctx);
  const names = { Videos: "videos", Musique: "music", Images: "images", Archives: "archives", Documents: "documents", Programmes: "programs" };
  for (const [rsName, jsName] of Object.entries(names)) check(`types : ${jsName} = apres.rs`, ctx.TYPES[jsName] === app[rsName], `${ctx.TYPES[jsName]} / ${app[rsName]}`);
}

// ---- Règle « même site » de common.js (référent et cookies d'un lien) ----
function sameSiteTests() {
  const ctx = { globalThis: {}, chrome: {}, URL };
  vm.createContext(ctx);
  vm.runInContext(fs.readFileSync(path.join(SRC, "common.js"), "utf8") + "\nthis.sameSite = sameSite;", ctx);
  const cases = [
    ["https://exemple.fr/a.zip", "https://exemple.fr/page", true],
    ["https://cdn.exemple.fr/a.zip", "https://exemple.fr/page", true],
    ["https://exemple.fr/a.zip", "https://cdn.exemple.fr/page", false],
    ["https://cdn.exemple.fr/a.zip", "https://www.exemple.fr/page", false],
    ["https://b.co.uk/a.zip", "https://a.co.uk/page", false],
    ["https://y.github.io/a.zip", "https://x.github.io/page", false],
    ["https://github.io/a.zip", "https://x.github.io/page", false],
    ["https://evilexemple.fr/a.zip", "https://exemple.fr/page", false],
    ["https://exemple.fr.evil.com/a.zip", "https://exemple.fr/page", false],
    ["magnet:?xt=urn:btih:abc", "https://exemple.fr/page", false],
    ["https://exemple.fr/a.zip", "", false],
    ["http://127.0.0.1:18990/a.zip", "http://127.0.0.1:18995/page", false],
    ["http://127.0.0.1:18990/a.zip", "http://127.0.0.1:18990/page", true],
    ["https://exemple.fr:443/a.zip", "https://exemple.fr/page", true],
  ];
  for (const [url, page, want] of cases) check(`même site : ${url} depuis ${page || "(rien)"} → ${want}`, ctx.sameSite(url, page) === want);
}

function testBuild() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "fluxdl-ext-"));
  fs.cpSync(SRC, dir, { recursive: true, filter: (p) => !p.includes(`${path.sep}dist`) });
  const m = JSON.parse(fs.readFileSync(path.join(dir, "manifest.json"), "utf8"));
  m.permissions.push("cookies"); // pas de dialogue de permission en headless
  m.host_permissions.push("<all_urls>");
  fs.writeFileSync(path.join(dir, "manifest.json"), JSON.stringify(m, null, 2));
  return dir;
}

// ---- Fausse appli ----
const many = [];
const single = [];
let manyCode = 200;
function fakeApp() {
  return http
    .createServer((req, res) => {
      const cors = { "Access-Control-Allow-Origin": req.headers.origin || "*", "Access-Control-Allow-Headers": "content-type,x-fluxdl-token" };
      if (req.method === "OPTIONS") return res.writeHead(204, cors).end();
      if (req.headers["x-fluxdl-token"] !== TOKEN) return res.writeHead(401, cors).end("{}");
      let body = "";
      req.on("data", (c) => (body += c));
      req.on("end", () => {
        if (req.url === "/ping") return res.writeHead(200, cors).end(JSON.stringify({ app: "fluxdl", version: "0.2.9" }));
        if (req.url === "/add-many") {
          if (manyCode !== 200) return res.writeHead(manyCode, cors).end("{}");
          const items = JSON.parse(body).items;
          many.push(items);
          return res.writeHead(200, cors).end(JSON.stringify({ ok: true, status: "received", count: items.length }));
        }
        if (req.url === "/add") {
          single.push(JSON.parse(body));
          return res.writeHead(200, cors).end(JSON.stringify({ ok: true, status: "added" }));
        }
        res.writeHead(404, cors).end("{}");
      });
    })
    .listen(APP_PORT, "127.0.0.1");
}

// ---- Page de test : ~30 liens (doublons, relatifs, javascript:, mailto:, ancres…) ----
const OTHER = `http://127.0.0.1:${WEB_PORT}`; // autre site (ni référent ni cookies)
const PAGE = `<!doctype html><meta charset="utf-8"><title>Fichiers de test</title>
<style>body{font:14px sans-serif;margin:20px} li{margin:2px 0}</style>
<h1>Téléchargements</h1>
<ul>
<li><a href="/files/setup-1.2.exe">Installeur Windows</a> <a href="/files/setup-1.2.exe">(miroir, doublon)</a>
<li><a href="files/paquet.zip">Archive relative</a> <a href="./files/paquet.zip">doublon relatif</a>
<li><a href="/files/manuel.pdf">Manuel PDF</a> <a href="/files/notes.txt">Notes</a>
<li><a href="/files/film.mkv">Film</a> <a href="/files/bande-annonce.mp4" download>Bande-annonce</a>
<li><a href="/files/album.flac">Album</a> <a href="/files/titre.mp3">Titre</a>
<li><a href="/files/photo.jpg">Photo</a>
<li><a href="/files/big.part1.rar">Partie 1</a> <a href="/files/big.part2.rar">Partie 2</a> <a href="/files/big.rar">RAR seul</a>
<li><a href="/files/app.dmg">Mac</a> <a href="/files/app.apk">Android</a>
<li><a href="${OTHER}/files/autre-site.iso">Image disque d'un autre site</a>
<li><a href="magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Ubuntu%2024.04">Magnet</a>
<li><a href="/docs/">Page de documentation</a> <a href="/about.html">À propos</a> <a href="/">Accueil</a>
<li><a href="#haut">Ancre</a> <a href="/page.html#bas">Ancre de la même page (adresse complète)</a> <a href="/about.html#equipe">Ancre d'une autre page</a>
<li><a href="javascript:void(0)">Script</a> <a href="mailto:a@b.example">Courriel</a> <a href="tel:+33100000000">Téléphone</a>
<li><a href="ftp://ftp.example/x.zip">FTP</a> <a href="data:text/plain,abc">Données</a>
</ul>
<img src="/img/logo.png" alt="Logo"> <img src="data:image/gif;base64,R0lGODlhAQABAAAAACw=" alt="pixel">
<video src="/media/clip.webm"></video>
<audio><source src="/media/son.ogg" type="audio/ogg"></audio>
<p id="sel">À copier : https://localhost:${WEB_PORT}/texte/liste.csv, et (magnet:?xt=urn:btih:fedcba9876543210fedcba9876543210fedcba98).
Lien de la sélection : <a href="/files/dans-selection.7z">7z sélectionné</a>.</p>
<p>Fin.</p>`;
function fakeSite() {
  return http
    .createServer((req, res) => {
      res.writeHead(200, { "Content-Type": "text/html; charset=utf-8", "Set-Cookie": "SID=secret-de-test; Path=/" });
      res.end(req.url.startsWith("/page.html") ? PAGE : "<!doctype html><title>x</title>");
    })
    .listen(WEB_PORT, "127.0.0.1");
}

(async () => {
  typeTableTests();
  sameSiteTests();
  const ext = testBuild();
  const app = fakeApp();
  const site = fakeSite();
  const ctx = await chromium.launchPersistentContext(fs.mkdtempSync(path.join(os.tmpdir(), "fluxdl-profil-")), {
    channel: "chromium",
    headless: true,
    viewport: { width: 900, height: 900 },
    locale: "fr-FR",
    args: [`--disable-extensions-except=${ext}`, `--load-extension=${ext}`, "--lang=fr"],
  });
  let [sw] = ctx.serviceWorkers();
  if (!sw) sw = await ctx.waitForEvent("serviceworker");
  const extId = new URL(sw.url()).host;
  await sw.evaluate(([t, p]) => chrome.storage.local.set({ token: t, port: p }), [TOKEN, APP_PORT]);

  const base = `http://localhost:${WEB_PORT}`;
  const page = await ctx.newPage();
  await page.goto(`${base}/page.html`);
  await page.waitForTimeout(500);
  const collect = (selection) =>
    sw.evaluate(async ([b, s]) => {
      const [tab] = await chrome.tabs.query({ url: b + "/page.html" });
      return chrome.tabs.sendMessage(tab.id, { type: "fluxdl-collect", selection: s }, { frameId: 0 });
    }, [base, selection]);

  // ---- 1. Collecte de toute la page ----
  const got = (await collect(false)).links.map((l) => l.url);
  const want = [
    "/files/setup-1.2.exe", "/files/paquet.zip", "/files/manuel.pdf", "/files/notes.txt", "/files/film.mkv",
    "/files/bande-annonce.mp4", "/files/album.flac", "/files/titre.mp3", "/files/photo.jpg", "/files/big.part1.rar",
    "/files/big.part2.rar", "/files/big.rar", "/files/app.dmg", "/files/app.apk",
  ].map((p) => base + p).concat([
    `${OTHER}/files/autre-site.iso`,
    "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Ubuntu%2024.04",
    `${base}/docs/`, `${base}/about.html`, `${base}/`, `${base}/about.html#equipe`,
    `${base}/files/dans-selection.7z`, `${base}/img/logo.png`, `${base}/media/clip.webm`, `${base}/media/son.ogg`,
  ]);
  check("collecte : liens attendus, absolus, sans doublon", JSON.stringify(got) === JSON.stringify(want), JSON.stringify(got.filter((u) => !want.includes(u)).concat(want.filter((u) => !got.includes(u)).map((u) => "manque " + u))));
  check("collecte : ni javascript:, mailto:, tel:, ftp:, data:", !got.some((u) => /^(javascript|mailto|tel|ftp|data):/.test(u)));
  check("collecte : pas d'ancre de la même page", !got.some((u) => u.startsWith(`${base}/page.html`)));
  check("collecte : liens en clair hors sélection ignorés", !got.some((u) => u.includes("liste.csv")));

  // ---- 2. Sélection : seulement ce qui est sélectionné (+ liens écrits en clair) ----
  await page.evaluate(() => {
    const r = document.createRange();
    r.selectNodeContents(document.getElementById("sel"));
    getSelection().removeAllRanges();
    getSelection().addRange(r);
  });
  const sel = (await collect(true)).links.map((l) => l.url);
  check(
    "sélection : lien + liens en clair (ponctuation retirée)",
    JSON.stringify(sel) === JSON.stringify([`${base}/files/dans-selection.7z`, `https://localhost:${WEB_PORT}/texte/liste.csv`, "magnet:?xt=urn:btih:fedcba9876543210fedcba9876543210fedcba98"]),
    JSON.stringify(sel)
  );

  // ---- 3. Page de choix ----
  const openPicker = async () => {
    const wait = ctx.waitForEvent("page");
    await sw.evaluate(async (b) => {
      const [tab] = await chrome.tabs.query({ url: b + "/page.html" });
      await openPicker(tab, false, 0);
    }, base);
    const p = await wait;
    await p.waitForLoadState();
    await p.waitForSelector("#list li");
    await p.setViewportSize({ width: 860, height: 900 });
    return p;
  };
  const pk = await openPicker();
  const rows = () => pk.locator("#list li").count();
  const checkedRows = () => pk.locator("#list input:checked").count();
  const count = () => pk.locator("#count").textContent();
  // Fichiers seulement par défaut (pages web cachées), images non cochées d'avance
  check("choix : 20 fichiers montrés (pages cachées)", (await rows()) === 20, String(await rows()));
  check("choix : images non cochées d'avance", (await checkedRows()) === 18, String(await checkedRows()));
  check("choix : compteur", (await count()) === "18 coché(s) sur 20 affiché(s)", await count());
  check("choix : bouton « Télécharger 18 fichiers »", (await pk.locator("#send").textContent()) === "Télécharger 18 fichiers");
  const first = await pk.locator("#list li").first().textContent();
  check("choix : nom deviné + domaine", first.includes("setup-1.2.exe") && first.includes("localhost"), first);
  await pk.screenshot({ path: path.join(CAP, "lotF_picker_fr.png"), fullPage: true });

  const typeBtn = (k) => pk.locator(`#types button[data-type="${k}"]`);
  const counts = {};
  for (const k of ["videos", "music", "images", "archives", "documents", "programs"]) {
    await typeBtn(k).click();
    counts[k] = await rows();
  }
  check("filtre type : vidéos 3, musique 3, images 2, archives 5, documents 2, programmes 4",
    JSON.stringify(counts) === JSON.stringify({ videos: 3, music: 3, images: 2, archives: 5, documents: 2, programs: 4 }), JSON.stringify(counts));
  await typeBtn("images").click();
  await pk.locator("#all").click();
  check("Tout cocher : sur les visibles seulement", (await count()) === "2 coché(s) sur 2 affiché(s)", await count());
  await typeBtn("all").click();
  check("Tout cocher n'a pas touché les autres", (await count()) === "20 coché(s) sur 20 affiché(s)", await count());
  await pk.locator("#exts").fill(".zip, pdf");
  check("filtre extension « .zip, pdf »", (await rows()) === 2, String(await rows()));
  await pk.locator("#exts").fill("*.part*.rar");
  check("filtre motif « *.part*.rar »", (await rows()) === 2, String(await rows()));
  await pk.locator("#exts").fill("");
  await pk.locator("#filter").fill("bande");
  check("filtre texte", (await rows()) === 1, String(await rows()));
  await pk.locator("#filter").fill("");
  await pk.locator("#none").click();
  check("Tout décocher", (await count()) === "0 coché(s) sur 20 affiché(s)" && (await pk.locator("#send").isDisabled()), await count());
  await pk.locator("#showAll").check();
  check("tous les liens : pages web en plus", (await rows()) === 24, String(await rows()));
  await pk.locator("#showAll").uncheck();
  await pk.locator("#all").click();
  await typeBtn("archives").click();
  await pk.screenshot({ path: path.join(CAP, "lotF_picker_archives_fr.png"), fullPage: true });
  await typeBtn("all").click();

  // ---- 4. Envoi groupé : un seul /add-many, référent + cookies du même site seulement ----
  await sw.evaluate(() => chrome.storage.local.set({ sendCookies: true }));
  await pk.locator("#send").click();
  await pk.waitForFunction(() => document.getElementById("msg").textContent.length > 0);
  const msg = await pk.locator("#msg").textContent();
  check("envoi : un seul /add-many de 20 liens, aucun /add", many.length === 1 && many[0].length === 20 && single.length === 0, `${many.length} ${many[0]?.length} ${single.length}`);
  const items = many[0] || [];
  const same = items.filter((i) => i.url.startsWith(base));
  check("envoi : même site → référent + cookies", same.length === 18 && same.every((i) => i.referrer === `${base}/page.html` && i.cookies === "SID=secret-de-test"), JSON.stringify(same[0]));
  const others = items.filter((i) => !i.url.startsWith(base));
  check("envoi : autre site et magnet → ni référent ni cookies", others.length === 2 && others.every((i) => !("referrer" in i) && !("cookies" in i)), JSON.stringify(others));
  check("envoi : message et lignes marquées", msg === "20 sur 20 envoyé(s) à fluxdl." && (await pk.locator("#list .state.ok").count()) === 20, msg);
  check("envoi : plus rien à envoyer", await pk.locator("#send").isDisabled());
  await pk.screenshot({ path: path.join(CAP, "lotF_picker_sent_fr.png"), fullPage: true });

  // ---- 5. Appli sans /add-many (404) : repli un par un (magnet exclu) ----
  manyCode = 404;
  const pk2 = await openPicker();
  await pk2.locator('#types button[data-type="archives"]').click();
  await pk2.locator("#send").click();
  await pk2.waitForFunction(() => document.getElementById("msg").textContent.length > 0);
  check("repli : 5 /add un par un", single.length === 5 && single.every((s) => s.url.startsWith(base) && s.referrer === `${base}/page.html`), String(single.length));
  manyCode = 200;
  await pk2.close();

  // ---- 6. Bouton du popup : ouvre la page de choix de l'onglet actif ----
  const tabId = await sw.evaluate(async (b) => (await chrome.tabs.query({ url: b + "/page.html" }))[0].id, base);
  const wait = ctx.waitForEvent("page");
  const popup = await ctx.newPage();
  await popup.goto(`chrome-extension://${extId}/popup.html`);
  check("popup : bouton présent", (await popup.locator("#grabLinks").textContent()) === "Récupérer les liens de la page");
  await popup.screenshot({ path: path.join(CAP, "lotF_popup_fr.png"), fullPage: true });
  const r = await popup.evaluate((id) => chrome.runtime.sendMessage({ type: "fluxdl-open-picker", tabId: id }), tabId);
  let opened = await wait;
  if (opened === popup) opened = await ctx.waitForEvent("page");
  await opened.waitForSelector("#list li");
  check("popup : page de choix ouverte", r?.ok === true && opened.url().includes("picker.html") && (await opened.locator("#list li").count()) === 20);

  await ctx.close();
  app.close();
  site.close();
  console.log(failures ? `${failures} échec(s)` : "tout est vert");
  console.log(`captures : ${CAP}`);
  process.exit(failures ? 1 : 0);
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
