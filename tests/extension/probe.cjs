// Extension 0.2.5, sonde des liens : le bouton flottant n'apparaît que si le lien donne un vrai
// fichier (pas une page web). Chromium Playwright (headless) + serveur local qui compte les requêtes.
// Usage : NODE_PATH=~/df/node_modules node tests/extension/probe.cjs
// Rien ne sort de 127.0.0.1 / localhost ; aucune appli nécessaire (le bouton ne l'appelle pas).
"use strict";
const { chromium } = require("playwright");
const fs = require("fs");
const os = require("os");
const path = require("path");
const http = require("http");

const SRC = path.join(__dirname, "..", "..", "extension");
const WEB_PORT = 18971;
let failures = 0;
const check = (name, ok, detail) => {
  console.log(`${ok ? "ok  " : "FAIL"} ${name}${detail ? " — " + detail : ""}`);
  if (!ok) failures++;
};

// Liens « fichiers » de la page et réponse de chacun
const LINKS = ["html", "zip", "attach", "slow", "nohead", "cf", "off"];
const hits = {}; // chemin → requêtes reçues ("HEAD", "GET bytes=0-0"…)
let cookieSent = false; // option « cookies » coupée : la sonde n'en envoie aucun
function server() {
  return http
    .createServer((req, res) => {
      const kind = req.url.split("/")[1];
      if (LINKS.includes(kind) && req.headers.cookie) cookieSent = true;
      (hits[kind] ??= []).push(`${req.method}${req.headers.range ? " " + req.headers.range : ""}`);
      const html = { "Content-Type": "text/html; charset=utf-8" };
      switch (kind) {
        case "":
          return res.writeHead(200, html).end(`<!doctype html><meta charset="utf-8"><title>liens</title>
<style>body{font:16px sans-serif;margin:40px} a{display:block;margin:0 0 40px;width:200px}</style>
${LINKS.map((k) => `<a id="${k}" href="/${k}/file.zip">${k}/file.zip</a>`).join("\n")}<p id="vide">vide</p>`);
        case "html": // hébergeur : page d'accueil derrière un lien .zip
          return res.writeHead(200, html).end("<html>page</html>");
        case "zip":
        case "off":
          return res.writeHead(200, { "Content-Type": "application/zip", "Content-Length": "4" }).end("PK..");
        case "attach": // page HTML servie en pièce jointe
          return res.writeHead(200, { ...html, "Content-Disposition": 'attachment; filename="r.html"' }).end("<p>x</p>");
        case "slow":
          return setTimeout(() => res.writeHead(200, { "Content-Type": "application/zip" }).end("PK.."), 4000);
        case "nohead": // HEAD refusé, GET Range accepté
          if (req.method === "HEAD") return res.writeHead(405).end();
          return res.writeHead(206, { "Content-Type": "application/octet-stream", "Content-Range": "bytes 0-0/4" }).end("P");
        case "cf": // défi anti-robot
          return res.writeHead(403, { ...html, "cf-mitigated": "challenge" }).end("<html>Just a moment...</html>");
        default:
          return res.writeHead(404).end();
      }
    })
    .listen(WEB_PORT, "127.0.0.1");
}

(async () => {
  const site = server();
  const ext = fs.mkdtempSync(path.join(os.tmpdir(), "fluxdl-ext-"));
  fs.cpSync(SRC, ext, { recursive: true, filter: (p) => !p.includes(`${path.sep}dist`) });
  const ctx = await chromium.launchPersistentContext(fs.mkdtempSync(path.join(os.tmpdir(), "fluxdl-profil-")), {
    channel: "chromium",
    headless: true,
    viewport: { width: 900, height: 700 },
    args: [`--disable-extensions-except=${ext}`, `--load-extension=${ext}`],
  });
  let [sw] = ctx.serviceWorkers();
  if (!sw) sw = await ctx.waitForEvent("serviceworker");
  await sw.evaluate(() => chrome.storage.local.set({ floatButton: true }));

  await ctx.addCookies([{ name: "SID", value: "secret", url: `http://localhost:${WEB_PORT}/` }]);
  const page = await ctx.newPage();
  await page.goto(`http://localhost:${WEB_PORT}/`);
  await page.waitForTimeout(500);
  const pill = page.locator("fluxdl-float");
  const visible = () => pill.isVisible().catch(() => false);
  const away = async () => {
    await page.hover("#vide");
    await page.waitForTimeout(900); // disparition après 700 ms
  };
  // Survol puis attente du bouton (ou de son absence) ; renvoie le délai d'apparition
  const hover = async (id, wait = 1500) => {
    await away();
    await page.hover(`#${id}`);
    const t0 = Date.now();
    while (Date.now() - t0 < wait) {
      if (await visible()) return Date.now() - t0;
      await page.waitForTimeout(50);
    }
    return -1;
  };

  check("lien .zip qui donne une page HTML : pas de bouton", (await hover("html")) < 0, JSON.stringify(hits.html));
  check("  (une seule requête HEAD, pas de GET)", JSON.stringify(hits.html) === '["HEAD"]');
  const tz = await hover("zip");
  check("lien .zip qui donne application/zip : bouton", tz >= 0, `${tz} ms`);
  check("page HTML en Content-Disposition attachment : bouton", (await hover("attach")) >= 0);
  check("HEAD refusé (405) puis GET Range 206 : bouton", (await hover("nohead")) >= 0, JSON.stringify(hits.nohead));
  check("  (GET avec Range: bytes=0-0)", JSON.stringify(hits.nohead) === '["HEAD","GET bytes=0-0"]');
  check("défi anti-robot 403 HTML : pas de bouton", (await hover("cf")) < 0, JSON.stringify(hits.cf));
  const t1 = Date.now();
  check("serveur lent (> 2,5 s) : pas de bouton", (await hover("slow", 4500)) < 0);
  check("  (abandon après 2,5 s, sans 2e requête)", (hits.slow || []).length === 1, JSON.stringify(hits.slow) + ` ${Date.now() - t1} ms`);

  check("sonde sans cookies (option coupée)", !cookieSent);

  // Cache : 2e survol = aucune nouvelle requête (positif et négatif)
  const before = JSON.stringify(hits);
  const tz2 = await hover("zip");
  check("cache positif : bouton tout de suite", tz2 >= 0 && tz2 < 300, `${tz2} ms`);
  check("cache négatif : toujours pas de bouton", (await hover("html", 800)) < 0);
  check("cache : aucune nouvelle requête", JSON.stringify(hits) === before);

  // Survols répétés pendant une sonde : une seule requête
  await sw.evaluate(() => probeCache.clear());
  hits.zip = [];
  await away();
  for (let i = 0; i < 5; i++) {
    await page.hover("#zip", { position: { x: 5 + i * 10, y: 5 } });
    await page.hover("#zip", { position: { x: 60 + i * 10, y: 8 } });
  }
  await page.waitForTimeout(800);
  check("sondes simultanées regroupées", JSON.stringify(hits.zip) === '["HEAD"]', JSON.stringify(hits.zip));

  // Pas plus de 4 sondes en parallèle, dédoublonnées
  const par = await sw.evaluate(async (base) => {
    probeCache.clear();
    let max = 0;
    const iv = setInterval(() => (max = Math.max(max, probeActive)), 5);
    const urls = Array.from({ length: 10 }, (_, i) => `${base}/zip/f${i}.zip`);
    const all = await Promise.all([...urls, ...urls].map(probeLink));
    clearInterval(iv);
    return { max, ok: all.every(Boolean), cache: probeCache.size };
  }, `http://localhost:${WEB_PORT}`);
  check("4 sondes en parallèle au plus", par.max > 0 && par.max <= 4 && par.ok && par.cache === 10, JSON.stringify(par));

  // Bouton coupé : aucune sonde
  await sw.evaluate(() => chrome.storage.local.set({ floatButton: false }));
  await page.reload();
  await page.waitForTimeout(500);
  check("bouton coupé : pas de bouton", (await hover("off", 1000)) < 0);
  check("bouton coupé : aucune requête", !hits.off, JSON.stringify(hits.off));
  check("arrière-plan : sonde refusée si le bouton est coupé", (await sw.evaluate((u) => handleMessage({ type: "fluxdl-probe", url: u }, { id: chrome.runtime.id, tab: { id: 1 }, url: u }), `http://localhost:${WEB_PORT}/off/file.zip`)).ok === false && !hits.off);

  await ctx.close();
  site.close();
  console.log(failures ? `${failures} échec(s)` : "tout est vert");
  process.exit(failures ? 1 : 0);
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
