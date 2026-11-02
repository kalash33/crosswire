# Coming from Burp Suite

Crosswire covers the daily core of a Burp workflow: capture, inspect, resend,
scope, decode. It is a younger and smaller tool, so this page maps what you know
to where it lives here and says plainly what is missing. For install and first
run, see the [README](../README.md).

## Where things are

| In Burp | In Crosswire | Notes |
|---|---|---|
| Proxy (intercept) | **Intercept** | Request intercept: hold, edit, forward, drop, forward-all. Starts disabled. Response intercept is not yet built (planned for 0.2). |
| Proxy > HTTP history | **History** | Every exchange, byte for byte, with Pretty, Raw, Headers and Hex tabs, search, filters, tags and notes. |
| Repeater | **Repeater** | Raw and assisted modes, tabs, a send history per tab. |
| Comparer | **Repeater: Compare with previous** | Line diff of two responses from the same tab (capped at 2 MiB a side). There is no free-form two-item comparer. |
| Target > Scope | **Scope** | Include and exclude rules; every exchange records its verdict. |
| Decoder | **Decoder** | A route of its own, and a pane for the selected message. |
| Intruder | Not yet | Planned for 0.2. |
| Extensions, BApp Store | Not yet | There is no extension API in 0.x. |
| Collaborator | Not planned for 0.x | |

## Setting up the CA

Like Burp, Crosswire reads HTTPS by terminating TLS with a certificate from its
own local CA, so a browser has to trust that CA. Two ways:

- **Let Crosswire launch the browser.** `crosswire app --project ./demo`, or
  `crosswire browser --project ./demo --proxy 127.0.0.1:8080` while `serve` is
  running, starts Chrome, Chromium, Edge or Brave with a dedicated profile that
  pins the CA. There is nothing to install and the system trust store is not
  touched.
- **Use your own browser.** Point its proxy at `127.0.0.1:8080`, open
  `http://crosswire/` through the proxy (the equivalent of Burp's
  `http://burpsuite`), download the CA and trust it. Steps for each browser and
  OS, including Firefox, Android and iOS, are in [ca-install.md](ca-install.md).

Crosswire never installs or removes the CA for you. The CA is generated per
machine and its key stays in your config directory (`crosswire ca path` prints
it).

## Three differences

1. **Byte-exact raw editing and diagnostics.** What you send is sent byte for
   byte, and what comes back is recorded exactly as received, so malformed and
   ambiguous HTTP survives the round trip. Where a message departs from the HTTP
   RFCs, the deviation is recorded as a diagnostic with the exact byte span
   instead of being silently fixed.
2. **Scope first, enforced.** Scope is part of the project, not an afterthought.
   Every exchange carries an in/out-of-scope verdict, History can filter and bulk
   add to scope, and with `--enforce-scope-strictly` the repeater refuses
   out-of-scope sends outright instead of recording them with an audit row.
3. **One binary, a local project.** The web UI is embedded in the `crosswire`
   binary, so there is no JVM, no separate UI to install and no account. A
   project is a directory (SQLite plus content packs) that survives a crash;
   copy it, archive it or open it again later.

## Keyboard shortcuts

Defaults follow Burp's where the browser allows it. Ctrl is the modifier on every
platform (not Cmd). Press `?` for the in-app sheet. Every one of these can be
remapped in **Settings**, which refuses browser-reserved chords, bare keys and
conflicts.

| Action | Default | Burp equivalent |
|---|---|---|
| Send to Repeater | Ctrl+R | Ctrl+R |
| Send (in Repeater) | Ctrl+Space | Ctrl+Space |
| Forward (Intercept) | Ctrl+F | Ctrl+F |
| Drop (Intercept) | Ctrl+D | Ctrl+D |
| Toggle intercept | Ctrl+I | Ctrl+T, which browsers reserve for a new tab |
| Decode selection | Ctrl+Shift+D | no default |
| Next / previous tab | Ctrl+Shift+] / Ctrl+Shift+[ | same |
| Copy as curl | Ctrl+Shift+C | Ctrl+Shift+C |
| Copy raw | Ctrl+Shift+S | |
| Shortcut sheet | ? | |

Inside a message viewer, Ctrl+F is the viewer's own find. The two copy shortcuts
copy the data unredacted, so a request can be replayed; the share-safe, redacted
variants are menu-only.

In History, `/` focuses the search box, and `j`, `k` and the arrow keys move
between rows.

## Where Crosswire is stricter

- **The destination gate.** After DNS resolution, the proxy and the repeater
  refuse loopback, link-local, cloud-metadata (for example `169.254.169.254`),
  unspecified, multicast and broadcast addresses, and Crosswire's own ports. If
  you are testing something on `localhost`, that is refused by default. Pass
  `--allow HOST:PORT` to `serve` (or `app`) to lift only the loopback and
  link-local refusals for that destination; each allowance is audited. Metadata
  addresses and Crosswire's own ports cannot be allowed at all.
- **Upstream certificates are verified.** A bad upstream certificate fails the
  request unless you name the host with `--accept-invalid-cert HOST`, which is
  recorded on the exchange and audited.
- **Intercept starts off,** and skips out-of-scope traffic and static file
  extensions by default.
- **Old pre-release projects are refused.** A project written by a pre-release
  build is refused on open, before anything is touched; re-record the traffic.

The design behind these rules, and the test that proves each one, is in
[security-gates.md](security-gates.md).
