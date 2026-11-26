// Page d'options : port, jeton, accès à l'appli, cookies.
"use strict";

const $ = (id) => document.getElementById(id);
const COOKIE_PERMS = { permissions: ["cookies"], origins: ["<all_urls>"] };

function showMsg(id, text, ok) {
  const el = $(id);
  el.textContent = text;
  el.className = "msg " + (ok ? "ok" : "err");
}

function readPort() {
  const n = Number($("port").value);
  return Number.isInteger(n) && n >= 1024 && n <= 65535 ? n : null;
}

async function saveConnection() {
  const port = readPort();
  if (port === null) return false;
  await saveSettings({ port, token: $("token").value.trim() });
  return true;
}

async function refreshAccess() {
  $("accessCard").hidden = await hasAppAccess();
}

function setTest(cls, text) {
  $("orb").className = "orb " + cls;
  $("testMsg").textContent = text;
}

async function testConnection() {
  if (!(await saveConnection())) {
    setTest("off", t("optPortInvalid"));
    return;
  }
  $("test").disabled = true;
  setTest("", t("optTesting"));
  const res = await ping();
  $("test").disabled = false;
  const texts = {
    unpaired: "statusNotPaired",
    noaccess: "statusNoAccess",
    down: "statusNotRunning",
    origin: "statusBadOrigin",
  };
  if (res.state === "ok") {
    setTest("on", t("statusConnected", [String(res.data?.version ?? "?")]));
  } else {
    setTest("off", t(texts[res.state] ?? "statusError"));
  }
}

async function init() {
  localizePage();
  const s = await getSettings();
  $("port").value = s.port;
  $("token").value = s.token;

  // Les cookies ne restent actifs que si la permission est encore là
  let cookiesOk = false;
  try {
    cookiesOk = await api.permissions.contains(COOKIE_PERMS);
  } catch {
    // API indisponible
  }
  $("sendCookies").checked = s.sendCookies && cookiesOk;

  $("port").addEventListener("change", () => {
    if (readPort() === null) setTest("off", t("optPortInvalid"));
    else saveConnection();
  });
  $("token").addEventListener("change", saveConnection);

  $("toggleToken").addEventListener("click", () => {
    const hidden = $("token").type === "password";
    $("token").type = hidden ? "text" : "password";
    $("toggleToken").textContent = t(hidden ? "optHide" : "optShow");
  });

  $("test").addEventListener("click", testConnection);

  // permissions.request doit être appelé directement dans le geste utilisateur
  $("grant").addEventListener("click", () => {
    api.permissions.request({ origins: [APP_ORIGIN] }).then(
      (granted) => {
        showMsg("accessMsg", t(granted ? "optAccessGranted" : "optPermDenied"), granted);
        if (granted) refreshAccess();
      },
      () => showMsg("accessMsg", t("optPermDenied"), false)
    );
  });

  $("sendCookies").addEventListener("change", (e) => {
    if (!e.target.checked) {
      saveSettings({ sendCookies: false });
      api.permissions.remove({ permissions: ["cookies"] }).catch(() => {});
      $("cookieMsg").textContent = "";
      return;
    }
    api.permissions.request(COOKIE_PERMS).then(
      (granted) => {
        e.target.checked = granted;
        saveSettings({ sendCookies: granted });
        showMsg("cookieMsg", granted ? t("saved") : t("optPermDenied"), granted);
      },
      () => {
        e.target.checked = false;
        saveSettings({ sendCookies: false });
        showMsg("cookieMsg", t("optPermDenied"), false);
      }
    );
  });

  // Capture, bouton flottant, listes de sites
  for (const id of ["autoCapture", "captureAll", "floatButton"]) {
    $(id).checked = !!s[id];
    $(id).addEventListener("change", async (e) => {
      await saveSettings({ [id]: e.target.checked });
      if (id !== "floatButton") showMsg("captureMsg", t("saved"), true);
    });
  }
  for (const id of ["allowSites", "denySites", "cookieSites"]) {
    $(id).value = parseList(s[id]).join("\n");
    $(id).addEventListener("change", async (e) => {
      const clean = parseList(e.target.value).join("\n");
      e.target.value = clean;
      await saveSettings({ [id]: clean });
      showMsg(id === "cookieSites" ? "cookieMsg" : "captureMsg", t("saved"), true);
    });
  }

  refreshAccess();
}

init();
