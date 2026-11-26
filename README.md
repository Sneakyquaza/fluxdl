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
  optional SHA-256 check and global speed limit.
- Benchmark tab: the same link with 1 connection, then N, to a temporary file deleted after.

## New in 0.2

- **In-app updates.** At launch and every 6 hours fluxdl reads the latest GitHub release in the
  background. A notification bubble offers **Update** or **Later**; Settings → Updates shows the
  installed version and a **Check now** button. On Windows the new `fluxdl.exe` is downloaded
  next to the app, checked against `fluxdl.exe.sha256` (GitHub hosts only, every redirect
  checked), swapped in and restarted. Running downloads pause and resume after the restart
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

### New in 0.2.3
- **Install the extension from the app**: Settings → Browser extension → **Install** (also
  offered once in a bubble at first launch). The extension is embedded in the exe. See
  [Install from fluxdl](#install-from-fluxdl-recommended).
- **Automatic pairing**: the extension asks, fluxdl shows "Allow?", no token to copy.
  See [Pairing](#pairing).

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
    # -> extension/dist/fluxdl-chrome-0.2.2.zip
    # -> extension/dist/fluxdl-firefox-0.2.2.zip

### Install on Chrome / Edge (by hand)
1. Open `chrome://extensions` (or `edge://extensions`).
2. Enable **Developer mode**.
3. Click **Load unpacked** and select the `extension/` folder (or the folder where you unzipped
   `fluxdl-chrome-0.2.2.zip`).

### Install on Firefox (by hand)
1. Unzip `fluxdl-firefox-0.2.2.zip` into a folder.
2. Open `about:debugging` → **This Firefox** → **Load Temporary Add-on…** and pick the
   `manifest.json` from that folder.
3. Open the extension options and click **Grant access to the app** (Firefox treats access to
   `127.0.0.1` as an optional permission).

Temporary add-ons are removed when Firefox restarts. A permanent install needs a Mozilla-signed
build, which is not provided yet. On Firefox Developer Edition or Nightly you can set
`xpinstall.signatures.required` to `false` in `about:config` and install the zip (renamed
`.xpi`) permanently.

### Pairing
Automatic (0.2.2): right after installation the extension asks fluxdl once (if it is running);
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
3. Click **Test connection**: the orb turns green with "Connected to fluxdl 0.2.2".

If fluxdl is not running (or not paired), **Download with fluxdl** falls back to a normal
browser download and shows a notification.

### Auto-capture
Off by default. Turn it on in the popup. A browser download (http/https only) is sent to
fluxdl when its size is at least the threshold (default 20 MB) or its extension is in the list
(default `zip,7z,rar,iso,exe,msi,dmg,mp4,mkv`). When fluxdl accepts it, the browser download is
cancelled; when fluxdl is not running, the download stays in the browser. Firefox often doesn't
report the size up front, so the extension list is what usually triggers there.

### Cookies (optional)
Some sites only serve a file to logged-in users. Enable **Send cookies** in the options to
forward that site's cookies to fluxdl with the link. The browser will ask for access to cookies
on all sites. Cookies are sent only to fluxdl on `127.0.0.1` (and kept with the download so it
can resume). Leave this off unless a site needs it.

## Build

```sh
cargo build --release                                     # Linux
cargo build --release --target x86_64-pc-windows-gnu      # Windows (needs mingw-w64)
cargo test
```

Command-line helpers: `fluxdl --benchmark URL [--connections 1,8,16] [--seconds 10]`,
`fluxdl --capture DIR` (renders the UI with demo data to PNG files).

Third-party: librqbit (Apache-2.0) for BitTorrent; yt-dlp (Unlicense) and ffmpeg (GPL build)
are separate programs, downloaded only with your consent and never bundled.

Font: Open Sans (SIL Open Font License, see `assets/fonts`), Latin subset. Background art: generated image, `assets/fond.jpg`.

## License

MIT
