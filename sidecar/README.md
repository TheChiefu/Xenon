# Running the sidecar

    cd sidecar && npm install

Start it from the directory that contains the server's `config.toml`:

    cd server && node ../sidecar/src/index.js

## Config

Keys the sidecar reads from `config.toml`, which the server writes on first run.
Anything missing is named in the startup error.

`[push]` and `[xbox]` each run a service by being in the file. Write one or
both.

```toml
[bind]
ip = "127.0.0.1"                     # connects to ws://ip:port/push/ws
port = 3000

[sidecar]
secret = "..."                       # sent as Authorization: Bearer

[push]
subject = "mailto:you@example.com"   # VAPID sub claim, mailto: or https://
ttl = 86400                          # seconds a push service stores an undelivered message

[xbox]
client_id = "..."                    # Entra app registration, personal accounts
client_secret = "..."                # Entra client secret value, not its id
redirect_uri = "https://example.com/xbox/callback"   # registered on that app, Web platform
poll_interval_seconds = 60
refresh_interval_seconds = 3600
```

## Files it writes

In the working directory, mode 0600:

- `vapid.json`: the VAPID keypair, generated on first start
- `subscriptions.json`: browser push subscriptions
- `xbox_links.json`: Xbox refresh tokens and the XUID mapping

Back these up. Losing `vapid.json` orphans every stored subscription.

## Files it reads

In the working directory:

- `game_titles.json`: what to show a game as, keyed on the name the service
  reports it under

```json
{
  "helldivers 2": "Helldivers 2",
  "halo: the master chief collection": "Halo MCC"
}
```

The sidecar removes `™`, `®` and `©` from every game name, then replaces a
listed name with its value, shown exactly as written. Matching ignores
capitalization, spacing, straight or curly quotes, and hyphen or dash.

A new entry takes effect on the next restart.
