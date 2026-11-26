# fluxdl

A free, open-source multi-connection download manager (Windows + Linux, single executable).

## How it works

- A probe request (`Range: bytes=0-0`) reads the size, range support, `ETag` / `Last-Modified`
  and the file name (`Content-Disposition`, else the URL).
- The file is split into up to 32 ranges downloaded in parallel (default 8).
- **Done early? Take half.** A connection that finishes its range takes the upper half of the
  largest remaining one, so no connection sits idle (minimum chunk: 1 MiB).
- Resume: data goes to a preallocated `.part` file; a small `.part.json` state file (ranges
  done) is written atomically every 0.5 s after the data is flushed to disk, so a crash or a
  power cut loses at most half a second. On resume the size and validators are checked: if the
  remote file changed, the download restarts cleanly.
- Every range response must be a `206` with the expected `Content-Range`; a server that answers
  `200` falls back to a single connection, never mixing bytes. Servers without ranges or with
  an unknown size use one connection.
- Per-connection retries with exponential backoff, redirects (8 max), HTTPS through rustls,
  optional SHA-256 check and global speed limit.
- Benchmark tab: the same link with 1 connection, then N, to a temporary file deleted after.

## Build

```sh
cargo build --release                                     # Linux
cargo build --release --target x86_64-pc-windows-gnu      # Windows (needs mingw-w64)
cargo test
```

Command-line helpers: `fluxdl --benchmark URL [--connections 1,8,16] [--seconds 10]`,
`fluxdl --capture DIR` (renders the UI with demo data to PNG files).

Font: Inter 4.1 (SIL Open Font License, see `assets/fonts`), Latin subset with tabular digits.

## License

MIT
