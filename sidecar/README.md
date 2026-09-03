# Sidecar

The sidecar is a Node service that connects to the server and handles the work
the server does not do itself. It sends web push notifications and polls Xbox for
account linking and presence. It reads the server's [config.toml](../server/config.md)
and writes its own files beside it.

To enable each service add one or more of the following headers:
- [push](#push)
- [xbox](#xbox)

Note: _A missing key is named in the startup error._

## Setup

```bash
cd sidecar && npm install
```

Start it from the directory that contains the server's `config.toml`:
```bash
cd server && node ../sidecar/src/index.js
```

## `[push]`

| Key | Example | Meaning |
|-|-|-|
| `subject` | `"mailto:you@example.com"` | VAPID sub claim, a `mailto:` or `https://` URL |
| `ttl` | `86400` | Seconds a push service stores an undelivered message |

Writes:
- `vapid.json`: the keypair, generated on first start
- `subscriptions.json`: the browser subscriptions

Losing `vapid.json` orphans every stored subscription.

## `[xbox]`

| Key | Example | Meaning |
|-|-|-|
| `client_id` | | Entra app registration, personal accounts |
| `client_secret` | | Entra client secret value |
| `redirect_uri` | `"https://example.com/xbox/callback"` | Registered on that app, Web platform |
| `poll_interval_seconds` | `60` | How often presence is polled |
| `refresh_interval_seconds` | `3600` | How often expiring tokens are refreshed |

Writes:
- `xbox_links.json`: Refresh tokens and XUID mapping

Reads:
- `games.json`: Keyed on reported name from Xbox

Example:
```json
{
  "helldivers_2": { "name": "Helldivers 2" },
  "halo_the_master_chief_collection": {
    "name": "Halo MCC",
    "activity": { "H: CE": "CE", "H: R": "Reach", "H2": "2" }
  }
}
```
Reported game names are normalized to `snake_case` for matching to the keyed
values. The keys are:
- `name` renames the game's title to the preferred value
- `activity` matches the start of the reported activity and replaces it, keeping
  whatever follows in parentheses
  - `H: R: Campaign - Normal` becomes `Reach (Campaign - Normal)`, and an
    activity that is only the prefix becomes `Reach`

A new entry takes effect on the next sidecar restart.

Games not listed in the file have `™`, `®` and `©` removed from their reported
name. A listed game shows its mapped `name` exactly as written.

