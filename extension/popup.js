// Popup : état de la connexion et réglages de capture.
"use strict";

const $ = (id) => document.getElementById(id);

function setStatus(cls, text) {
  $("orb").className = "orb " + cls;
  $("status").textContent = text;
}

async function refreshStatus() {
  const res = await ping();
  $("pairRow").hidden = res.state === "ok" || res.state === "noaccess";
  switch (res.state) {
    case "ok":
      setStatus("on", t("statusConnected", [String(res.data?.version ?? "?")]));
      break;
    case "unpaired":
      setStatus("off", t("statusNotPaired"));
      break;
    case "noaccess":
      setStatus("off", t("statusNoAccess"));
      break;
    case "down":
      setStatus("off", t("statusNotRunning"));
      break;
    case "origin":
      setStatus("off", t("statusBadOrigin"));
      break;
    default:
      setStatus("off", t("statusError"));
  }
}

let savedTimer;
async function save(part) {
  await saveSettings(part);
  $("saved").textContent = t("saved");
  clearTimeout(savedTimer);
  savedTimer = setTimeout(() => ($("saved").textContent = ""), 1200);
}

async function init() {
  localizePage();
  const s = await getSettings();
  $("autoCapture").checked = s.autoCapture;
  $("thresholdMB").value = s.thresholdMB;
  $("extensions").value = s.extensions;

  $("autoCapture").addEventListener("change", (e) => save({ autoCapture: e.target.checked }));
  for (const id of ["captureAll", "floatButton"]) {
    $(id).checked = !!s[id];
    $(id).addEventListener("change", (e) => save({ [id]: e.target.checked }));
  }
  // Le badge « ! » (capture rendue au navigateur) a été vu
  Promise.resolve(api.action.setBadgeText({ text: "" })).catch(() => {});
  $("thresholdMB").addEventListener("change", (e) => {
    const n = Math.round(Number(e.target.value));
    if (!Number.isFinite(n) || n < 1) {
      e.target.value = s.thresholdMB;
      return;
    }
    e.target.value = n;
    save({ thresholdMB: n });
  });
  $("extensions").addEventListener("change", (e) => {
    const clean = parseList(e.target.value).join(",");
    e.target.value = clean;
    save({ extensions: clean });
  });
  $("openOptions").addEventListener("click", () => {
    api.runtime.openOptionsPage();
    window.close();
  });

  // Appairage : la réponse se donne dans l'appli (le popup peut se fermer, l'arrière-plan
  // continue et garde le jeton)
  $("pair").addEventListener("click", async () => {
    $("pair").disabled = true;
    setStatus("", t("pairWaiting"));
    const res = await api.runtime.sendMessage({ type: "fluxdl-pair" });
    $("pair").disabled = false;
    setStatus(res?.state === "paired" ? "on" : "off", pairText(res?.state));
    if (res?.state === "paired") $("pairRow").hidden = true;
  });

  // Diagnostic : version de l'extension (pas seulement de l'appli) et dernier échec vidéo
  const diag = [t("popupExtVersion", [api.runtime.getManifest().version])];
  const { lastVideo } = await api.storage.local.get("lastVideo");
  if (lastVideo?.reason) {
    const at = new Date(lastVideo.at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    diag.push(t("popupLastVideoError", [at, lastVideo.reason]));
  }
  $("diag").textContent = diag.join(" · ");

  refreshStatus();
}

init();
