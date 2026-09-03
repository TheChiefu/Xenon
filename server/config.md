# Configuration

Every key in `config.toml`, with the default the server writes on first run. For
where the file is written, see the [README](../README.md#configuration).

The first value checked is `version`, which is the format the configuration file
was written for. If it doesn't match the version the server supports then the
server refuses to start.

## `[push]` and `[xbox]`

Must be manually added to file (never written by server):
- Adding `[push]` turns on push notifications
- Adding `[xbox]` turns on Xbox account linking

The keys for each are documented in [../sidecar/README.md](../sidecar/README.md).

## `[info]`

How `GET /server` reports the server instance.

| Key | Default | Meaning |
|-|-|-|
| `name` | `Xenon Server` | Shown to clients as the server's name |
| `kind` | `Development` | The type of server (ie. a development server, release, etc.) |
| `description` | `My custom Xenon server` | Longer text shown alongside the name |

## `[bind]`

| Key | Default | Meaning |
|-|-|-|
| `ip` | `127.0.0.1` | Address the listener binds |
| `port` | `3000` | Port the listener binds |
| `certificate` | _unset_ | Path to a PEM certificate chain served to clients |
| `key` | _unset_ | Path to the PEM private key for the certificate |

Setting `certificate` and `key` serves HTTPS directly. Setting one without the
other is refused at startup. Using neither starts the server using HTTP.

## `[storage]`

| Key | Default | Meaning |
|-|-|-|
| `files` | `files` | Directory holding uploaded files, sharded by hash |

## `[database]`

| Key | Default | Meaning |
|-|-|-|
| `path` | `chat.db` | SQLite database file |
| `max_connections` | `5` | Most connections open at once |
| `min_connections` | `0` | Connections held open while unused |
| `acquire_timeout_seconds` | `30` | How long a request waits for a free connection |
| `idle_timeout_seconds` | `600` | How long an unused connection stays open, `0` to keep it open |
| `busy_timeout_seconds` | `5` | How long a statement waits for the write lock |

## `[logging]`

| Key | Default | Meaning |
|-|-|-|
| `file` | `xenon.log` | File every tracing call is appended to, empty for stdout only |
| `level` | `info` | Lowest level written |

## `[session]`

| Key | Default | Meaning |
|-|-|-|
| `lifetime_days` | `30` | How long a session survives without activity |
| `renew_after_days_elapsed` | `1` | How often an active session's expiry is rewritten |

Activity always extends expiry to `lifetime_days` from now.
`renew_after_days_elapsed` limits how often that write happens, and must be less
than `lifetime_days`.

## `[sidecar]`

| Key | Default | Meaning |
|-|-|-|
| `secret` | empty | What a sidecar sends to connect |

`secret` is the authentication key the sidecar uses when it connects to the server.
The server knows the secret and a sidecar connecting needs to use it to make a connection.
See more information here at: [README.md](../sidecar/README.md).

## `[limits]`

| Key | Default | Meaning |
|-|-|-|
| `username_min` | `2` | Shortest username |
| `username_max` | `32` | Longest username |
| `display_name_min` | `1` | Shortest display name |
| `display_name_max` | `64` | Longest display name |
| `profile_description_max` | `2000` | Longest profile description |
| `room_name_max` | `128` | Longest room name |
| `message_body_max` | `8000` | Longest message body |
| `password_min` | `8` | Shortest password |
| `password_max` | `128` | Longest password |
| `file_bytes_max` | `"25MiB"` | Largest single upload |
| `attachments_per_message_max` | `10` | Most attachments on one message |
| `message_page` | `200` | Messages returned per page |
| `room_page` | `200` | Rooms returned per page |
| `users_page` | `25` | Users returned per page |
| `message_buffer` | `32` | Events queued for the sidecar connection before the oldest is dropped |

Lengths are in characters. `file_bytes_max` is a string with a unit suffix,
accepting either decimal or binary units such as: "MiB", "KB", "GiB", etc.
Using a value without a suffix defaults to reading in bytes.
