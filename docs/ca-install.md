# Trusting the Crosswire CA

Crosswire reads HTTPS by terminating the browser's TLS connection and
presenting a certificate it signs itself. A browser accepts that certificate
only if it trusts the Crosswire certificate authority (CA). This page covers
getting the CA and the quickest ways to make a browser trust it.

The CA is generated the first time you run `crosswire serve` or
`crosswire ca export`, and is unique to your machine. It lives in two files,
`ca.crt` and `ca.key`, in the directory `crosswire ca path` prints
(`~/.config/crosswire/ca` on Linux, `~/Library/Application Support/crosswire/ca`
on macOS), or in the directory you pass as `--ca-dir`. Its subject is
`CN=Crosswire CA Crosswire CA`, which is the name to look for in a certificate
store.

> **Not tested end to end.** Apart from the Chromium key-pin route, which
> Crosswire's browser test runs, the steps on this page were written from each
> platform's and browser's documentation and have not been run as part of
> Crosswire's own tests. Check what your platform shows.

> **Trust it only where you test.** Anyone who holds `ca.key` can issue
> certificates that every browser trusting this CA will accept for any site.
> Use a separate browser profile or a test device, keep the key out of backups
> you share, and remove the CA when you finish. Crosswire does not install or
> remove it for you.

## Get the CA

Any one of these gives you the certificate (and the key pin, below).

- **From the proxy.** With the browser or `curl` going through the proxy, open
  `http://crosswire/`. It serves a short page and these files:

  | path | content |
  | --- | --- |
  | `/ca.der` | the certificate, DER; most system and browser stores want this |
  | `/ca.pem` | the certificate, PEM; command-line tools and Firefox accept this |
  | `/ca.spki` | the base64 SHA-256 pin of the CA's public key, one line |

  ```
  curl -x http://127.0.0.1:8080 -o crosswire-ca.der http://crosswire/ca.der
  ```

  The page is plain HTTP and is served to anything that can reach the
  listener, proxy authentication or not. Through the proxy, open
  `http://crosswire/`. A request sent straight to the listener (not as a proxy
  request) also gets the page when its `Host` names the proxy: `crosswire`,
  `crosswire:PORT`, the listener's own address as bound, or `localhost:PORT`
  when the listener is on loopback. For a wildcard bind such as `0.0.0.0:8080`
  the address as bound is never the one a client uses (the second line
  `crosswire serve` prints then names an address that does not work), so use a
  direct request with `Host: crosswire`, or go through the proxy. On a phone,
  set the proxy first, then open `http://crosswire/`.
- **From the command line.** `crosswire ca export --format der --out
  crosswire-ca.der` (or `--format pem`). Without `--out` the certificate goes to
  stdout. `crosswire ca path` says where the files are, and
  `crosswire ca spki` prints the pin.

## Chromium, Chrome and Edge: trust by key pin

The quickest route for desktop Chromium-based browsers needs no installation.
Start the browser with the proxy and the CA's pin:

```
chromium \
  --proxy-server=127.0.0.1:8080 \
  --ignore-certificate-errors-spki-list="$(crosswire ca spki)" \
  --user-data-dir=/tmp/crosswire-profile
```

On macOS the binary is `/Applications/Google Chrome.app/Contents/MacOS/Google
Chrome`; on Windows it is `chrome.exe`. The throwaway `--user-data-dir` keeps
the test browser apart from your own. Chromium shows a banner saying the flag is
unsupported; that is expected. Chromium does not send loopback traffic through a
proxy unless you also pass `--proxy-bypass-list="<-loopback>"`, and the
destination gate refuses loopback targets unless you allowed them with
`crosswire serve --allow HOST:PORT`.

The pin identifies the CA key, so it stays valid for as long as the CA files do.
If you delete the CA directory, a new CA (and pin) is generated.

## System trust

Chrome and Edge on macOS and Windows use the system store, so installing the CA
there covers them and most other native software. These commands change what
your machine trusts; run them only on a test machine (and see the note at the
top: they have not been run as part of Crosswire's own tests).

### macOS

```
sudo security add-trusted-cert -d -r trustRoot \
  -k /Library/Keychains/System.keychain crosswire-ca.pem
```

Or open `crosswire-ca.der` in Keychain Access, add it to the System or login
keychain, open it, expand Trust and choose "Always Trust" for SSL. To remove it,
delete the `Crosswire CA Crosswire CA` certificate in Keychain Access.

### Linux

Debian, Ubuntu:

```
sudo cp crosswire-ca.pem /usr/local/share/ca-certificates/crosswire-ca.crt
sudo update-ca-certificates
```

Fedora, RHEL:

```
sudo cp crosswire-ca.pem /etc/pki/ca-trust/source/anchors/crosswire-ca.pem
sudo update-ca-trust
```

Arch and other p11-kit systems: `sudo trust anchor crosswire-ca.pem`.

This covers `curl`, Python and most command-line tools. Chrome and Chromium on
Linux read the per-user NSS database instead, so for them either use the pin
above or add the CA with `certutil` (from `libnss3-tools`):

```
certutil -d sql:$HOME/.pki/nssdb -A -t "C,," -n crosswire -i crosswire-ca.pem
```

To remove the system copy, delete the file you added and run the same update
command again.

### Windows

Open `certmgr.msc`, expand Trusted Root Certification Authorities, right-click
Certificates, choose All Tasks, then Import, and select `crosswire-ca.der`. This
trusts it for your user. From a command prompt, the same for your user is:

```
certutil -user -addstore Root crosswire-ca.der
```

An elevated prompt with `certutil -addstore Root crosswire-ca.der` trusts it for
the whole machine. To remove it, delete `Crosswire CA Crosswire CA` from the same
list.

## Firefox

Firefox keeps its own store and ignores the system one by default.

1. Settings, Privacy & Security, Certificates, View Certificates, the
   Authorities tab, Import.
2. Choose `crosswire-ca.pem` (or the DER file).
3. Tick "Trust this CA to identify websites" and confirm.

Then point Firefox at the proxy under Settings, Network Settings, Manual proxy
configuration: HTTP proxy `127.0.0.1`, port `8080`, and "Also use this proxy for
HTTPS". Firefox keeps `localhost, 127.0.0.1` in its "No proxy for" list by
default; clear it to record loopback targets, and allow them with `--allow`.
Remove the CA from the same Authorities list when you are done.

## Android

Install the certificate under Settings, Security (or Security & privacy),
Encryption and credentials, Install a certificate, CA certificate, and pick
`crosswire-ca.der`.

**From Android 7 (API 24) apps do not trust user-installed CAs by default.** An
app trusts one only if it opts in through its network security configuration
(a debuggable build can do this with `<debug-overrides>`), so installing the CA
this way does not let Crosswire read most apps' HTTPS. Browsers set their own
policy: some trust user CAs and some need a setting, so test the one you use. A
CA installed into the system store of a rooted device or an emulator image is
trusted by everything, and is outside what Crosswire does.

To send a device's traffic through Crosswire, the proxy must listen on a
reachable address, which means `--allow-non-loopback` and `--proxy-auth`, and the
device's Wi-Fi proxy (manual) must point at that address and port. From the
Android emulator the host is `10.0.2.2`.

## iOS

1. Set the device's Wi-Fi proxy (Configure Proxy, Manual) to the proxy's address
   and port; the proxy must listen on a reachable address (`--allow-non-loopback`
   and `--proxy-auth`).
2. In Safari, open `http://crosswire/` and tap `/ca.der`. Allow the
   configuration profile download.
3. Settings, General, VPN & Device Management (or Profile Downloaded), the
   Crosswire profile, Install.
4. **Settings, General, About, Certificate Trust Settings, and turn on "Enable
   full trust" for the Crosswire CA.** Installing the profile alone is not
   enough: the certificate is not trusted for websites until this switch is on.

Apps that pin their certificates will still refuse the proxy's.

## Checking it worked

Browse to an HTTPS site through the proxy. If the page loads, the proxy's
certificate was accepted and the exchange is in the project. If the browser shows
a certificate warning, or a page fails to load, the CA is not trusted yet. The
proxy notices: it counts each rejected handshake (the `handshake_rejections`
counter) and, after three for one host within 30 seconds, writes an `Error`
marker, "Browser rejected the Crosswire certificate for HOST — is the CA
trusted?". It does not stop intercepting that host. To leave a host alone on
purpose (a site that pins its certificate, for example), start the proxy with
`--passthrough HOST` (or `--passthrough .example.com` for its subdomains); the
connection is then tunnelled untouched and recorded only as a byte count.

## Starting over

Delete the CA directory (`crosswire ca path`) to get a new CA and key pin the
next time you run `crosswire serve`. Remove the old certificate from every store
you added it to; the new one has to be trusted again.
