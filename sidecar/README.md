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

- `games.json`: how a game is shown, keyed on the reported name

```json
{
  "helldivers_2": { "name": "Helldivers 2" },
  "halo_the_master_chief_collection": {
    "name": "Halo MCC",
    "activity": { "H: CE": "CE", "H: R": "Reach", "H2": "2" }
  }
}
```

The sidecar removes `™`, `®` and `©` from every game name.
Reported game names are normalized to `snake_case` and matched
against the file to the server owner's preferred name.

`activity` matches and renames the game activity's presence also to a preferred value.
Converting values like:  `H: R: Campaign - Normal` to `Reach (Campaign - Normal)`.

A new entry takes effect on the next sidecar restart.
