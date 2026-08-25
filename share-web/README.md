# Sivtr share web

This is the independent browser viewer and encrypted publication service. It
does not share the Astro/Starlight documentation build.

There is one production path: the Node 22 self-hosted adapter behind Nginx
(currently `share.hnnulwh.cn`). The Cloudflare Worker + R2 adapter is an
optional second deployment for networks that can reach Cloudflare. It is not
a runtime fallback. Point `[publish].endpoint` at the origin you actually
run; the CLI will not try the other backend if that origin is down. The
Wrangler `proxy` environment is experimental and still requires Cloudflare
reachability from the origin, so it is not a mainland-China fallback.

Both adapters expose the same opaque `/api/v1` contract.

Only the encrypted `SIVTPUB1` envelope and deletion/expiry metadata are
stored. The title, provider, refs, `cwd`, content hash, plaintext, viewing key,
and raw management token never cross the API boundary or enter server logs.

## Local verification

```powershell
npm.cmd ci
npm.cmd run typecheck
npm.cmd run test
npm.cmd run build
npm.cmd run dry-run
```

Use npm with the committed `package-lock.json`, matching CI. Do not add a
`bun.lock`. Cloudflare deploys are optional and require account secrets
plus an approved GitHub Environment; they are not required for the
self-hosted production path.

## R2 lifecycle

Apply [`r2-lifecycle.json`](./r2-lifecycle.json) to the `sivtr-publications`
bucket through the Cloudflare R2 lifecycle API before staging. The Worker
still checks the exact `expires_at` on every GET/DELETE; lifecycle cleanup is
only the eventual physical-delete safety net.

## Self-hosting `share.hnnulwh.cn`

The local adapter requires only Node 22. It binds to `127.0.0.1:8791`; do not
expose that port in the server firewall. Nginx terminates public TLS and
forwards requests locally. Envelopes are created atomically under
`/var/lib/sivtr-share/v1/<expiry-class>/`; each `.bin` has a small adjacent
`.json` containing only the management-token SHA-256, creation/expiry times,
and envelope version. Exact expiry is enforced on every read and a periodic
cleanup removes expired or incomplete pairs.

```bash
useradd --system --home /var/lib/sivtr-share --shell /usr/sbin/nologin sivtr-share
install -d -o root -g root -m 0755 /opt/sivtr-share/server /opt/sivtr-share/dist
install -d -o sivtr-share -g sivtr-share -m 0700 /var/lib/sivtr-share

# Copy server/self-host.mjs, dist/, and the unit before these commands.
install -o root -g root -m 0644 deploy/systemd/sivtr-share.service /etc/systemd/system/sivtr-share.service
systemctl daemon-reload
systemctl enable --now sivtr-share
```

Before a certificate exists, install the HTTP bootstrap Nginx config. After
DNS points `share.hnnulwh.cn` to the server, install the certificate and key
under `/etc/nginx/ssl/share.hnnulwh.cn/` with root ownership and mode `0600`,
then replace the bootstrap with
[`deploy/nginx/share.hnnulwh.cn.conf`](./deploy/nginx/share.hnnulwh.cn.conf).
Always run `nginx -t` before reload. Point Sivtr at this origin in
`config.toml`:

```toml
[publish]
endpoint = "https://share.hnnulwh.cn"
```

The CLI default for `[publish].endpoint` is empty so a crate install does
not send publications to an operator-specific host.

The current Alibaba Cloud DV certificate is not managed by Certbot. Renew and
redeploy it before its expiry date; replacing the two files followed by
`nginx -t && systemctl reload nginx` is sufficient and does not require a
publication-service restart.

## Domain and kill switch

Configure DNS and TLS for `share.hnnulwh.cn` before production. Set
`Environment=CREATE_ENABLED=false` in a systemd override and restart the
service to stop new publications during an abuse or cost incident while
leaving reads and revocations available. For optional Cloudflare
deployments, use the equivalent Worker variable. Changing
`[publish].endpoint` does not migrate existing objects.
