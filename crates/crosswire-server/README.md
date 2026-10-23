# crosswire-server

The `crosswire` binary: it runs the MITM proxy from
[`crosswire-proxy`](https://github.com/kalash33/crosswire/blob/main/crates/crosswire-proxy/README.md)
against a project directory from
[`crosswire-store`](https://github.com/kalash33/crosswire/blob/main/crates/crosswire-store/README.md),
and manages the interception CA. Part of
[Crosswire](https://github.com/kalash33/crosswire/blob/main/README.md), an open-source Rust web security testing platform.

With this binary the project is a headless recorder: point a browser at the
proxy, browse, and every exchange lands in the project, ready for the UI that
comes in alpha.2.

## What it does and does not do

- `crosswire serve` opens (or creates) a project, loads (or generates) the CA,
  binds the proxy listener, records into the store, audits its configuration,
  and stops cleanly on Ctrl-C or SIGTERM.
- `crosswire ca export`, `ca spki` and `ca path` give you the CA certificate,
  its Chromium pin, and where it lives.
- Does **not** have a UI, an API, a repeater, intercept, scope, or a
  `crosswire browser` launcher. Those are alpha.2.
- Does **not** install the CA into any operating system, browser or keychain; the
  CA is a plain file store. See
  [docs/ca-install.md](https://github.com/kalash33/crosswire/blob/main/docs/ca-install.md).
- Does **not** switch hosts to passthrough on its own. `--passthrough` is manual.
- Unix only for now, because `crosswire-store` is.

## Commands

```
crosswire [--log LEVEL] <COMMAND>

  serve   Run the intercepting proxy, recording into a project
  ca      Inspect or export the interception CA
```

`--log` is global and defaults to `info`. `RUST_LOG`, when set and valid, wins.
Logs go to **stderr**; stdout carries command output only.

### `crosswire serve`

| flag | default | meaning |
| --- | --- | --- |
| `--project PATH` | required | project directory, created if missing |
| `--proxy-bind ADDR` | `127.0.0.1:8080` | listen address; port 0 picks a free port |
| `--allow-non-loopback` | off | permit a non-loopback bind (needs `--proxy-auth`) |
| `--proxy-auth USER:PASS` | none | require `Proxy-Authorization: Basic`; split at the first `:`, both parts non-empty |
| `--passthrough HOST` | none | tunnel this host, or `.suffix`, without interception; repeatable |
| `--allow HOST:PORT` | none | let the proxy reach a destination the gate refuses (loopback, link-local, cloud metadata); repeatable; `[v6]:port` for IPv6 |
| `--accept-invalid-cert HOST` | none | accept an invalid upstream certificate from this host; repeatable; audited |
| `--ca-dir PATH` | the platform config dir (`crosswire ca path`) | where `ca.crt` and `ca.key` live |
| `--capture-cap BYTES` | 52428800 (50 MiB) | bytes of each body kept; the rest is relayed only |

The proxy's other limits keep their defaults; they are in the
[`crosswire-proxy` README](https://github.com/kalash33/crosswire/blob/main/crates/crosswire-proxy/README.md).

Startup runs in this order, so a bad configuration creates nothing:

1. Build the proxy configuration and validate it (a refused non-loopback bind, a
   bad passthrough entry). Nothing exists on disk yet.
2. Open or create the project and open the store. If the previous run did not
   close cleanly, a warning is logged; the store has already written the gap
   marker.
3. Load the CA, or generate one when neither `ca.crt` nor `ca.key` exists
   (audit `ca.generate`).
4. Bind the listener, then build the destination gate so it refuses the proxy's
   own port even when `--proxy-bind` asked for port 0.
5. Audit each `--allow` (`allow.add`) and each `--accept-invalid-cert`
   (`cert.override.config`).
6. Start the proxy and audit `listener.bind`.
7. Print two lines on stdout, then serve until a signal arrives.
8. On shutdown, log the counters, audit `listener.stop`, and close the store.
   `Store::close` runs on every path once the store is open, so the project is
   marked cleanly closed even after an error.

The audit log is `audit.jsonl` in the project, actor `cli`, result `ok`. The
actions are `ca.generate` (only when the CA was created), `allow.add`,
`cert.override.config`, `listener.bind` and `listener.stop`. `ca export` and
`ca spki` write no audit rows: there is no project to write them to.

### `crosswire ca`

| command | does |
| --- | --- |
| `ca export --format der\|pem [--out PATH] [--ca-dir PATH]` | writes the CA certificate to the file, or to stdout; generates the CA when there is none |
| `ca spki [--ca-dir PATH]` | prints the base64 SHA-256 of the CA's SubjectPublicKeyInfo and a newline, for Chromium's `--ignore-certificate-errors-spki-list` |
| `ca path` | prints the default CA directory, `<config dir>/crosswire/ca` (`~/.config/crosswire/ca` on Linux, `~/Library/Application Support/crosswire/ca` on macOS) |

### Exit codes

| code | meaning |
| --- | --- |
| 0 | a clean stop on Ctrl-C or SIGTERM, or a command that succeeded |
| 2 | bad usage or configuration: a clap error, a non-loopback bind without `--allow-non-loopback` or without `--proxy-auth`, a bad `--passthrough`, `--accept-invalid-cert`, `--allow`, `--proxy-auth` or `--log`, or no config directory |
| 1 | a runtime failure: opening the project or store, CA I/O, binding, writing an audit row or stdout, closing the store |

Errors are printed to stderr as `crosswire: error: ...`.

## Example

```text
$ crosswire serve --project ./demo
proxy listening on 127.0.0.1:8080
ca page at http://crosswire/ (via the proxy) or http://127.0.0.1:8080/
```

Logs, on stderr (each line also carries a timestamp, omitted here):

```text
INFO crosswire::serve: proxy listening addr=127.0.0.1:8080
```

From another terminal, with the browser (or `curl -x`) pointed at
`127.0.0.1:8080`:

```text
$ curl -s -x http://127.0.0.1:8080 http://crosswire/ca.spki
sFSsabiQ82cQbtbCpDqRxuUtAp3pbbJe6OMDbuWb/do=
$ crosswire ca export --format pem --out crosswire-ca.pem
```

Ctrl-C stops it (the counts depend on what you browsed):

```text
^C
INFO crosswire::serve: shutting down
INFO crosswire::serve: proxy stopped accepted=5 exchanges=0 tunnels=0
$ echo $?
0
```

The project then holds `project.db`, `packs/`, `journal/`, `project.lock` and
`audit.jsonl`; the format is in
[docs/format.md](https://github.com/kalash33/crosswire/blob/main/docs/format.md).

## Security notes

- The proxy binds loopback by default. A non-loopback bind needs both
  `--allow-non-loopback` and `--proxy-auth`; the credentials are visible in the
  process arguments, so use this on a host you control.
- `--allow` and `--accept-invalid-cert` loosen the destination gate and upstream
  certificate checks for named hosts only, and each is written to the audit log.
- The CA private key (`ca.key`, 0600) is the most sensitive file Crosswire
  creates: anyone holding it can mint certificates your browser trusts. Keep it
  out of project directories and backups you share, and remove the CA from any
  browser or system store when you finish testing.
- A project records credentials, cookies and tokens exactly as they crossed the
  proxy. Treat it as sensitive data; redaction applies when you export.

## Status

Pre-alpha, `publish = false`, version `0.0.0` in `Cargo.toml`; this is the
binary of the `0.1.0-alpha.1` headless recorder. The command line is not frozen.

Design: [spec section 12](https://github.com/kalash33/crosswire/blob/main/docs/superpowers/specs/2026-10-22-crosswire-design.md)
(configuration and `crosswire serve`) and
[ADR-0003](https://github.com/kalash33/crosswire/blob/main/docs/adr/0003-proxy-relay.md).
