// Popup : état de la connexion et réglages de capture.
"use strict";

const $ = (id) => document.getElementById(id);

function setStatus(cls, text) {
  $("orb").className = "orb " + cls;
  $("status").textContent = text;
}

async function refreshStatus() {
  const res = await ping();
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
    const clean = e.target.value
      .split(",")
      .map((x) => x.trim().replace(/^\./, "").toLowerCase())
      .filter(Boolean)
      .join(",");
    e.target.value = clean;
    save({ extensions: clean });
  });
  $("openOptions").addEventListener("click", () => {
    api.runtime.openOptionsPage();
    window.close();
  });

  refreshStatus();
}

init();
