// Sites vidéo : sur leurs pages de vidéo, un bouton « ⬇ fluxdl » près du lecteur envoie le lien
// de la PAGE à l'appli (yt-dlp le lit ; le <video> y est un flux blob: inutilisable). Partagé par
// l'arrière-plan, le script de contenu et les options. Ajouter un site = une entrée ici :
//   id            clé du réglage « cookies » (videoCookies[id])
//   hosts         domaines (sous-domaines compris)
//   page(u)       vrai si l'URL (objet URL) est une page de vidéo
//   cookieDomains domaines dont les cookies de connexion servent à yt-dlp (option par site)
//   player        sélecteurs du lecteur, dans l'ordre (le bouton se pose dans son coin)
"use strict";

// Twitch : premiers segments qui ne sont pas des chaînes
const TWITCH_RESERVED = new Set(
  ("directory,videos,settings,search,downloads,jobs,p,store,turbo,subscriptions,inventory,wallet," +
    "drops,friends,messages,prime,login,signup,following,popout,embed,moderator,u,team,broadcast")
    .split(",")
);

const VIDEO_SITES = [
  {
    id: "youtube",
    name: "YouTube",
    hosts: ["youtube.com", "youtu.be"],
    page: (u) =>
      u.hostname === "youtu.be"
        ? /^\/[\w-]{6,}$/.test(u.pathname)
        : (u.pathname === "/watch" && /^[\w-]{6,}$/.test(u.searchParams.get("v") || "")) ||
          /^\/(live|shorts)\/[\w-]{6,}\/?$/.test(u.pathname),
    // yt-dlp ne lit que les cookies de youtube.com (SAPISID, LOGIN_INFO…), pas ceux de google.com
    cookieDomains: ["youtube.com"],
    player: ["#movie_player", "ytd-player", "#player", "video"],
  },
  {
    id: "twitch",
    name: "Twitch",
    hosts: ["twitch.tv"],
    page: (u) => {
      const p = u.pathname;
      if (u.hostname === "clips.twitch.tv") return /^\/[\w-]{3,}\/?$/.test(p);
      // Rediffusion : /videos/N, ancienne forme /<chaîne>/v/N ou /<chaîne>/video/N
      if (/^\/(videos|\w+\/v(ideo)?)\/\d+\/?$/.test(p) || /^\/\w+\/clip\/[\w-]+\/?$/.test(p)) return true;
      // Chaîne : enregistre le live en cours (yt-dlp refuse si la chaîne est hors ligne)
      const m = /^\/(\w{3,25})\/?$/.exec(p);
      return !!m && !TWITCH_RESERVED.has(m[1].toLowerCase());
    },
    // yt-dlp lit le cookie auth-token de .twitch.tv
    cookieDomains: ["twitch.tv"],
    player: ['[data-a-target="video-player"]', ".video-player", "video"],
  },
];

function hostMatches(host, domain) {
  return host === domain || host.endsWith("." + domain);
}

// Site vidéo de l'URL (page de vidéo ou non), sinon null
function videoSiteOf(url) {
  let u;
  try {
    u = new URL(url);
  } catch {
    return null;
  }
  if (u.protocol !== "https:" && u.protocol !== "http:") return null;
  const host = u.hostname.toLowerCase();
  return VIDEO_SITES.find((s) => s.hosts.some((d) => hostMatches(host, d))) ?? null;
}

// Site si l'URL est une page de vidéo, sinon null
function videoPageSite(url) {
  const site = videoSiteOf(url);
  if (!site) return null;
  try {
    return site.page(new URL(url)) ? site : null;
  } catch {
    return null;
  }
}

// Motifs d'URL des sites (menus contextuels, permissions)
function siteOrigins(site, domains) {
  return (domains ?? site.hosts).flatMap((d) => [`*://${d}/*`, `*://*.${d}/*`]);
}
