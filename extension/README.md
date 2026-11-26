# fluxdl browser extension

Manifest V3 extension for Chrome, Edge and Firefox that sends downloads to the fluxdl app
running on `127.0.0.1`. Plain JavaScript, no build step, no remote code.

## Answers from the app (0.2.3)

- `POST /add` now answers after fluxdl decided: `added` or `already_active` (200, the browser
  copy may be dropped), `already_done` or `refused` (409) and `timeout` (503): the browser keeps
  its download. Auto-capture sends `probe: true`: fluxdl checks the link gives the file (with
  the referrer and, if enabled, the cookies) before accepting; a login page is refused.
- The floating button's style is re-applied each time it appears, and a click only counts if
  the button is really visible under the pointer (a page making it transparent can't use it).
- Firefox 128 or later (optional host permissions).

## Pairing (0.2.2)

- **Connect to fluxdl** (popup and options) or, once, right after installation: the background
  sends `POST /pair {nonce}` (no token) and repeats it every second with the same nonce while
  fluxdl shows "Allow?". On **Allow** the app answers with the token, once; the extension stores
  it and shows "Paired ✓". Denied, busy (another request pending), expired (60 s) and "app not
  running" are reported. Manual token pairing stays available in the options.
- Only the extension's own pages can start a pairing (message `fluxdl-pair`, sender checked).
  On Firefox the options page asks for the `127.0.0.1` permission in the same click.

## Features (0.2.1)

- **Auto-capture** (off by default, popup or options): a browser download is handed to fluxdl
  when it is at least the size threshold, matches the extension list, comes from an
  "always capture" site, or **Capture everything** is on. "Never capture" sites win over all
  other rules. The address sent is the final one after redirects, with the referrer and the
  suggested file name. On Chrome/Edge the browser waits (`downloads.onDeterminingFilename`)
  until fluxdl answers; on Firefox the download is paused meanwhile. The browser copy is
  cancelled and erased **only after fluxdl accepted the link**; otherwise the browser keeps
  the download, a notification says "fluxdl not running — downloaded by the browser" and the
  toolbar icon shows a red `!` (cleared when the popup opens). `blob:` and `data:` downloads
  always stay in the browser (fluxdl can't fetch them).
- **Floating button** (on by default): hovering a link to a likely file (archives, disk images,
  installers, media, PDF…, or a link with a `download` attribute) or a `<video>`/`<audio>` with
  a real address shows a small "⬇ fluxdl" button. It lives in a closed shadow root, positioned
  absolutely, so the page's CSS can't alter it and the layout doesn't move. Buttons that start
  a download from a script (no link) don't get one: auto-capture handles those.
- **Right-click menu**: "Download with fluxdl" on links, "Download this media with fluxdl" on
  videos and audio, "Download all links on this page with fluxdl…" on pages (opens a picker
  with checkboxes; file-like links are pre-selected).
- **Cookies** (optional): sent only if enabled, and only for the listed sites (empty list = all
  sites).

## Security

- Only the background script talks to fluxdl and only it knows the pairing token. The content
  script never reads extension storage; it sends `{type, url, title}` messages to the background,
  which checks the sender is this extension, accepts only `http(s)` URLs (≤ 8 KiB), and uses the
  frame's real URL as referrer (not one claimed by the page).
- Clicks on the floating button must be real user clicks (`isTrusted`); a page can't trigger it
  by script.
- The picker data and the "send" action are accepted only from the extension's own pages.

## Permissions

| Permission | Why |
| --- | --- |
| `downloads` | Watch browser downloads for auto-capture, cancel/erase them once fluxdl took over, and fall back to a browser download when fluxdl isn't running. |
| `contextMenus` | The three right-click entries. |
| `storage` | Settings (port, token, capture rules). `storage.session` keeps the picker's link list for the tab it opens. |
| `notifications` | "fluxdl not running", errors. |
| host `http://127.0.0.1/*` | Talk to the fluxdl app on this computer (optional on Firefox: granted in the options). |
| content script on `http://*/*`, `https://*/*` | **New in 0.2.1.** Needed for the hover button and to list a page's links for the picker. It only reads link and media addresses when you hover or ask for the picker, and sends nothing anywhere except to the extension's background. This is why the browser now says the extension can "read and change data on websites". |
| optional `cookies` + `<all_urls>` | Only requested when you turn on **Send cookies**. |

No `tabs`, `scripting`, `activeTab` or `webRequest` permission is used.
