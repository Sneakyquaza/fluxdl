# fluxdl

A free, open-source multi-connection download manager (Windows + Linux, single executable).

## How it works

- A probe request (`Range: bytes=0-0`) reads the size, range support, `ETag` / `Last-Modified`
  and the file name (`Content-Disposition`, else the URL).
- The file is split into up to 32 ranges downloaded in parallel (default 8).
- **Done early? Take half.** A connection that finishes its range takes the upper half of the
  largest remaining one, so no connection sits idle (minimum chunk: 1 MiB).
- Resume: data goes to a `.part` file at its final size; a small `.part.json` state file (ranges
  done) is written atomically every 0.5 s after the data is flushed to disk, so a crash or a
  power cut loses at most half a second. On resume the size and validators are checked: if the
  remote file changed, the download restarts cleanly.
- The `.part` file is sparse (NTFS sparse flag on Windows): Explorer shows the final size right
  away, but disk space is only used as data arrives, and Windows never has to fill gigabytes of
  zeros before a write far into the file. Free space is checked before starting.
- Every range response must be a `206` whose `Content-Range` starts where asked, ends within the
  range and has the expected total; nothing is ever written past the file size. A connection
  that gets `200`, a wrong range, an HTML page or `403`/`429` gives its range back; if the host
  keeps refusing extra connections they are closed automatically (the reason shows in the
  detail panel) and the others carry on. A server that ignores ranges for every connection falls
  back to a single connection. Servers without ranges or with an unknown size use one connection.
- Pause and cancel return within a second even if the server stalls. Cancel asks whether to
  delete the partial file. At launch, a download whose `.part` was deleted is not recreated:
  it shows "Partial file not found" with Start over / Remove.
- Per-connection retries with exponential backoff, redirects (8 max), HTTPS through rustls,
  optional SHA-256 check and a per-download speed limit.
- Benchmark tab: the same link with 1 connection, then N, to a temporary file deleted after.

## New in 0.2

- **In-app updates.** At launch and every 6 hours fluxdl reads the latest GitHub release in the
  background. A notification bubble offers **Update** or **Later**; Settings → Updates shows the
  installed version and a **Check now** button. On Windows the new `fluxdl.exe` is downloaded
  next to the app, checked against `fluxdl.exe.sha256` (GitHub hosts only, every redirect
  checked) and, since 0.2.3, against an Ed25519 signature (`fluxdl.exe.sig`) made with a key
  that never leaves the maintainer's machine (the public key is built into the app), then
  swapped in and restarted. Running downloads pause and resume after the restart
  (you are asked first). On Linux the bubble opens the release page.
- **Videos (yt-dlp).** Paste a YouTube (or other site) link, or retry a link that turned out to
  be a web page: fluxdl runs `yt-dlp -J` and offers best quality (up to 8K), 1080p, 720p, or
  audio only (M4A / MP3) with estimated sizes. yt-dlp downloads, ffmpeg merges picture and
  sound. On Windows, first use offers to download the official `yt-dlp.exe`
  (github.com/yt-dlp/yt-dlp, checked against `SHA2-256SUMS`) and the shared ffmpeg build
  (github.com/yt-dlp/FFmpeg-Builds, checked against `checksums.sha256`) into the app data
  folder; yt-dlp updates itself weekly. On Linux the system `yt-dlp` and `ffmpeg` are used.
  YouTube may need a JavaScript runtime (deno or node) installed for all formats.
- **Torrents and magnet links** (librqbit, Apache-2.0): paste a magnet link, open or drop a
  `.torrent` file, pick the files, follow progress, peers, download and upload speed and a
  pieces map. Sharing after the download is off by default (optional ratio). Torrents resume
  after a restart (pieces already on disk are re-checked); an existing file that fluxdl did
  not create is never overwritten. On Windows fluxdl can open magnet links and `.torrent`
  files (asked on the first torrent, toggle in Settings, current user only).
- **Browser extension** (below).

### New in 0.2.4
- **YouTube and Twitch from the browser.** On a video page (youtube.com/watch, /live/, /shorts/,
  youtu.be, twitch.tv/videos/…, Twitch clips, a live Twitch channel) the extension shows a
  "⬇ fluxdl" button in the corner of the player, and "Download this video with fluxdl" in the
  right-click menu. It sends the **page** link (the player itself plays a `blob:` stream that
  can't be downloaded); fluxdl opens the quality window. YouTube and Twitch change pages
  without reloading: the button follows (YouTube's navigation event, back/forward, and the
  address re-read once a second on those sites only).
- **Subscriber-only / members-only videos and VODs.** Options → Videos → "Send my login cookies
  for this site", one switch per site, **off by default**. Only that site's cookies are sent
  (YouTube: `youtube.com`, which is all yt-dlp reads; Twitch: `twitch.tv`), only to fluxdl on
  `127.0.0.1`, from the tab's own cookie store (private window, Firefox container).
  See [Login cookies for videos](#login-cookies-for-videos).
- **Format window**: "Section only: from … to …" (hh:mm:ss, needs ffmpeg, cut at the nearest
  keyframe; the file name gets the times) and, for a YouTube live in progress, "Live: from its
  beginning" (`--live-from-start`). Other lives (Twitch) are recorded from now until they end
  or until Pause.
- **Settings → Videos → Default video quality**: pre-selected in the format window, and used
  without asking for a video sent by the extension while fluxdl is minimized.
- Pause on a long VOD stops yt-dlp; Resume continues its `.part` (`--continue`). A section or a
  live starts over instead.

### New in 0.2.3
- **Install the extension from the app**: Settings → Browser extension → **Install** (also
  offered once in a bubble at first launch). The extension is embedded in the exe. See
  [Install from fluxdl](#install-from-fluxdl-recommended).
- **Automatic pairing**: the extension asks, fluxdl shows "Allow?", no token to copy.
  See [Pairing](#pairing).
- **Security and reliability fixes** from an independent audit (42 findings, see
  `out/audit` locally):
  - Windows tools (`explorer`, `reg`, `taskkill`, `tar`) are started by their full path, from a
    safe folder: a file named `explorer.exe` in Downloads is never run.
  - yt-dlp runs with `--ignore-config` (and without plugins for the app's own copy): a
    `yt-dlp.conf` dropped in Downloads is ignored.
  - Only one fluxdl runs: a second launch hands its link to the first one (or brings it to the
    front) and exits.
  - Downloads, extension links, the clipboard watcher and saving keep working while the window
    is minimized or hidden.
  - The extension gives up the browser's copy only when fluxdl really took the link
    (`added` / `already_active`); an auto-captured link is checked first (a login page stays in
    the browser).
  - Torrent file names that could leave the download folder (`C:`, `..`, `:`, `NUL`…) are refused.
  - Downloaded files get the Windows "from the Internet" mark (Zone.Identifier).
  - Compressed (`gzip`) responses, expired links ("Update the link"), weak ETags, redirects
    with cookies, empty files and dangerous file names are handled; error messages never show
    the link (signed links carry tokens).
  - A `queue.json` entry this version can't read is kept, not wiped (`queue.json.bak`).
  - Greek and Cyrillic names display; Chinese, Japanese and Korean use the system font.
  - Minimum window 900 × 600; no CPU used at rest apart from the tips card fading every 12 s.

### Extension features (0.2.1)
- Auto-capture: clicking a site's download button hands the real file (final URL after redirects) to fluxdl; the browser download is cancelled only once fluxdl confirms. If fluxdl isn't running, the browser keeps the download and shows a badge.
- Floating "⬇ fluxdl" button when hovering file links and videos (toggle in the popup).
- Right-click: "Download with fluxdl" on links, videos and audio; "Download all links on this page…" opens a picker.
- Options: capture everything, always/never-capture site lists, per-site cookie sending (off by default).

## Browser extension

fluxdl ships a Manifest V3 extension for Chrome, Edge and Firefox (`extension/`). It adds a
**Download with fluxdl** entry to the right-click menu on links, videos and audio, and can
optionally capture browser downloads automatically.

The app listens on `127.0.0.1:47591` only (port configurable). Every request must carry the
random pairing token shown in fluxdl's Settings; requests from web pages (any `http(s)` origin)
and from other host names are refused, and requests are rate limited.

### Install from fluxdl (recommended)
The extension is not on the browser stores yet. fluxdl carries it and guides you:

- **Chrome, Edge, Brave, Opera, Vivaldi**: **Install** writes the extension to
  `%LOCALAPPDATA%\fluxdl\extension` (Linux: `~/.config/fluxdl/extension`), detects the
  installed browsers (known install folders and the Windows `App Paths` registry; `PATH` on
  Linux), copies the folder path and opens the browser's extensions page. Then: turn on
  **Developer mode**, click **Load unpacked**, paste the path. The folder is rewritten when
  fluxdl updates: click **Reload** on the extension's card (or restart the browser) to pick up
  the new version.
- **Firefox**: fluxdl saves `fluxdl-firefox-<version>.zip` in your download folder, copies its
  path and opens `about:debugging#/runtime/this-firefox`. Click **Load Temporary Add-on…** and
  pick the zip. Firefox removes temporary add-ons when it closes, until the extension is signed
  by Mozilla.

If a browser refuses to open its internal page from the command line, type the address shown
in the guide (`chrome://extensions`, `edge://extensions`, `brave://extensions`…).

Build the packages by hand (optional, needs only `bash` and `python3`):

    ./extension/package.sh
    # -> extension/dist/fluxdl-chrome-0.2.3.zip
    # -> extension/dist/fluxdl-firefox-0.2.3.zip

### Install on Chrome / Edge (by hand)
1. Open `chrome://extensions` (or `edge://extensions`).
2. Enable **Developer mode**.
3. Click **Load unpacked** and select the `extension/` folder (or the folder where you unzipped
   `fluxdl-chrome-0.2.3.zip`).

### Install on Firefox (by hand)
1. Unzip `fluxdl-firefox-0.2.3.zip` into a folder (Firefox 128 or later).
2. Open `about:debugging` → **This Firefox** → **Load Temporary Add-on…** and pick the
   `manifest.json` from that folder.
3. Open the extension options and click **Grant access to the app** (Firefox treats access to
   `127.0.0.1` as an optional permission).

Temporary add-ons are removed when Firefox restarts. A permanent install needs a Mozilla-signed
build, which is not provided yet. On Firefox Developer Edition or Nightly you can set
`xpinstall.signatures.required` to `false` in `about:config` and install the zip (renamed
`.xpi`) permanently.

### Pairing
Automatic (0.2.3): right after installation the extension asks fluxdl once (if it is running);
otherwise click **Connect to fluxdl** in the popup or the options. fluxdl shows "A fluxdl
extension asks for access (Chrome, origin chrome-extension://…). Allow?". Click **Allow**: the
extension receives the token and shows "Paired ✓".

How it is protected: `POST /pair` needs no token but only an extension origin
(`chrome-extension://` or `moz-extension://`; web pages, requests without an origin and other
host names are refused). The extension repeats the request with the same random nonce until you
answer; the token goes only to that request, once, and only after your click. One request at a
time (others get "busy"), 3 new requests per minute at most, and a request expires after
60 seconds. Escape or closing the dialog means **Deny**.

By hand (fallback, or with an older fluxdl):
1. Start fluxdl and open **Settings → Browser extension**: click **Copy** next to the pairing
   token.
2. Open the extension options (popup → **Options**), paste the token, keep the port at `47591`
   unless you changed it in the app.
3. Click **Test connection**: the orb turns green with "Connected to fluxdl 0.2.3".

If fluxdl is not running (or not paired), **Download with fluxdl** falls back to a normal
browser download and shows a notification.

### Auto-capture
Off by default. Turn it on in the popup. A browser download (http/https only) is sent to
fluxdl when its size is at least the threshold (default 20 MB) or its extension is in the list
(default `zip,7z,rar,iso,exe,msi,dmg,mp4,mkv`). fluxdl first checks that the link gives the
file (not a login page); only when it accepts it is the browser download cancelled. When fluxdl
is not running, refuses the link or already has it, the download stays in the browser. Firefox often doesn't
report the size up front, so the extension list is what usually triggers there.

### Cookies (optional)
Some sites only serve a file to logged-in users. Enable **Send cookies** in the options to
forward that site's cookies to fluxdl with the link. The browser will ask for access to cookies
on all sites. Cookies are sent only to fluxdl on `127.0.0.1` (and kept with the download so it
can resume). Leave this off unless a site needs it.

### Login cookies for videos
For a subscriber-only Twitch VOD or a members-only YouTube video, yt-dlp must be logged in as
you. Turn on **Send my login cookies for this site** for that site in the extension options
(the browser asks for access to that site's cookies only). Then:
- the extension reads that site's cookies when you click the button or the menu, and sends
  them to fluxdl on `127.0.0.1` with the page link (never to the Internet);
- fluxdl keeps them **in memory only** (never in `queue.json`, never in a log), and for each
  yt-dlp run writes them to a temporary Netscape cookies file in its data folder
  (`cookies/`, readable by you only: `0600` on Linux, your profile's `%APPDATA%` on Windows),
  passed with `--cookies` and deleted as soon as yt-dlp exits (also on Pause/Cancel). Files
  left by a crash are deleted at the next start;
- pausing and resuming in the same session reuses them. After fluxdl restarts they are gone:
  a video that needed them says "Login cookies are gone… click the fluxdl button on the video
  page again", and that click resumes it (the `.part` is kept).

Anyone holding these cookies is logged in as you: leave the switch off on a shared computer.
YouTube may rotate the cookies of a browser session that yt-dlp used; if YouTube logs you out
in the browser, log in again.

## Build

```sh
cargo build --release                                     # Linux
cargo build --release --target x86_64-pc-windows-gnu      # Windows (needs mingw-w64)
cargo test
```

Command-line helpers: `fluxdl --benchmark URL [--connections 1,8,16] [--seconds 10]`,
`fluxdl --capture DIR` (renders the UI with demo data to PNG files), `fluxdl URL|magnet|file.torrent`
(handed to the running fluxdl if there is one).

Releases: `cargo run --example signer -- signer fluxdl.exe VERSION KEY.pk8` writes
`fluxdl.exe.sig`; publish `fluxdl.exe`, `fluxdl.exe.sha256`, `fluxdl.exe.sig` and
`THIRD-PARTY.txt` (regenerate it with `python3 -I packaging/third_party.py > THIRD-PARTY.txt`).

Third-party: librqbit (Apache-2.0) for BitTorrent; yt-dlp (Unlicense) and ffmpeg (GPL build)
are separate programs, downloaded only with your consent and never bundled.

Fonts: Open Sans (SIL Open Font License), Latin, Greek and Cyrillic subset; DejaVu Sans
(Bitstream Vera license) for arrows and symbols; see `assets/fonts`. Notices of the Rust crates
built into the exe: `THIRD-PARTY.txt`. Background art: generated image, `assets/fond.jpg`.

## License

MIT
