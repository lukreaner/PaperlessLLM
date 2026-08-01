# Reverse proxy and LAN access

PaperlessLLM binds to `127.0.0.1` by default. That is the safest setting when the dashboard is used only on the host. Remote access should use HTTPS through a reverse proxy or a private VPN; never publish the plain HTTP listener directly to the internet.

## Security prerequisites

Before exposing the dashboard:

1. Configure the single-administrator password in **Settings > LAN security** and verify that a private browser session is asked to sign in.
2. Keep the PaperlessLLM listener on `127.0.0.1` when the proxy runs on the same host.
3. Use a valid TLS certificate and forward the original `Host` and protocol headers.
4. Allow only the intended network at the firewall or VPN layer.
5. Do not put Paperless or AI provider tokens in proxy configuration, URLs, or access logs.

The `/healthz` route intentionally remains unauthenticated and contains only service status, semantic version, and opaque build/data-directory fingerprints used by the desktop launcher. It contains no configuration or credentials. Do not treat it as an application API.

## Caddy

Caddy obtains and renews public certificates automatically when DNS and ports 80/443 are available:

```caddyfile
paperless-llm.example.com {
    encode zstd gzip

    reverse_proxy 127.0.0.1:8765 {
        header_up Host {host}
        header_up X-Forwarded-Proto {scheme}
        header_up X-Forwarded-Host {host}
        header_up X-Forwarded-For {remote_host}
    }

    header Strict-Transport-Security "max-age=31536000"
}
```

Use Caddy's internal CA or an explicitly configured certificate for private-only names. Install that CA on every client instead of bypassing certificate checks.

## nginx

The following server blocks redirect HTTP to HTTPS and proxy the application on loopback:

```nginx
map $http_upgrade $connection_upgrade {
    default upgrade;
    ''      close;
}

server {
    listen 80;
    listen [::]:80;
    server_name paperless-llm.example.com;
    return 301 https://$host$request_uri;
}

server {
    listen 443 ssl;
    listen [::]:443 ssl;
    http2 on;
    server_name paperless-llm.example.com;

    ssl_certificate     /etc/letsencrypt/live/paperless-llm.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/paperless-llm.example.com/privkey.pem;

    client_max_body_size 25m;
    add_header Strict-Transport-Security "max-age=31536000" always;

    location / {
        proxy_pass http://127.0.0.1:8765;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-Host $host;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection $connection_upgrade;
        proxy_read_timeout 300s;
        proxy_send_timeout 300s;
    }
}
```

Use `nginx -t` before reloading. Add HSTS only after HTTPS is working reliably and the hostname will remain HTTPS-only.

## Proxy on another computer

When the reverse proxy cannot reach loopback, bind PaperlessLLM to a private interface instead of `0.0.0.0` where possible. Configure LAN authentication first, then firewall port 8765 so only the proxy's source address can connect. TLS must still terminate at the proxy, and the direct backend port must not be reachable by ordinary clients.

## Validation

After deployment, confirm all of the following from a different computer:

- `http://` redirects to `https://`.
- The TLS certificate is trusted and matches the hostname.
- A private browser session must authenticate.
- Login, logout, form submissions, and the review approval flow work without CSRF or cookie errors.
- The browser developer console shows no mixed-content or cross-origin failures.
- Port 8765 is not reachable directly from the client network.

PaperlessLLM's custom CA setting applies to its outbound connection to a private Paperless-ngx server. It does not configure the browser-facing reverse-proxy certificate.
