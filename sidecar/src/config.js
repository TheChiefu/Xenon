// Reads config.toml, the file the server writes on first run. The server
// owns the defaults; the sidecar reads the keys the server already writes
// where they exist, and validates the rest.

import { readFileSync } from 'node:fs';
import { parse } from 'smol-toml';

const CONFIG_FILE = 'config.toml';
const SIDECAR_PATH = '/sidecar/ws';

// Every key the sidecar reads, by section, with the type it has to be.
// Nothing is filled in from here: the server writes config.toml and owns the
// defaults, so a key that is absent is a mistake to report rather than a
// value to invent. Anything unchecked fails much further in: an absent
// push.subject reaches web-push, and an interval that was quoted in the TOML
// reaches setInterval as NaN, which runs the timer continuously.
const SECTIONS = {
  bind: [['ip', 'string'], ['port', 'number']],
  sidecar: [['secret', 'string']],
  push: [['subject', 'string'], ['ttl', 'number']],
  xbox: [
    ['client_id', 'string'],
    ['client_secret', 'string'],
    ['redirect_uri', 'string'],
    ['poll_interval_seconds', 'number'],
    ['refresh_interval_seconds', 'number'],
  ],
};

// What the sidecar needs to connect at all.
const REQUIRED_SECTIONS = ['bind', 'sidecar'];

// One service per section. Each runs when its section is in the file, so a
// deployment that wants only notifications writes only [push].
const SERVICE_SECTIONS = ['push', 'xbox'];

/// Reads and validates config.toml, returning the parsed file with one
/// added top-level key, `url`: the WebSocket address to connect to, built
/// from `[bind]` rather than configured separately, since the sidecar
/// connects to the same address the server listens on.
///
/// `[push]` and `[xbox]` each turn a service on by being in the file, and at
/// least one of them has to be. A section that is there is validated in
/// full, so four of five Xbox keys is a typo to report rather than a choice
/// to leave Xbox off.
///
/// Throws if the file is absent, if no service section is in it, if any key
/// of a section being read is missing or of the wrong type, or if
/// `[push] subject` is not a `mailto:` or `https://` URL. Every bad key is
/// named in one message, so filling the file in by hand does not take one
/// run per key.
///
/// - `path`: file to read, relative to the working directory both
///   processes are started from.
export function readConfig(path = CONFIG_FILE) {
  let text;
  try {
    text = readFileSync(path, 'utf8');
  } catch (e) {
    if (e.code === 'ENOENT') {
      throw new Error(`${path} not found, the server creates it on first run, start it first`);
    }
    throw e;
  }

  const config = parse(text);

  const services = SERVICE_SECTIONS.filter((section) => config[section] !== undefined);
  if (services.length === 0) {
    throw new Error(`${path} configures no services, add ${SERVICE_SECTIONS.map((s) => `[${s}]`).join(' or ')}`);
  }

  const bad = [...REQUIRED_SECTIONS, ...services].flatMap((section) =>
    SECTIONS[section]
      .filter(([key, type]) => {
        const value = config[section]?.[key];
        // An empty string counts as unset: it is what the server writes for
        // a secret it has not generated yet, and it refuses every connection.
        return typeof value !== type || (type === 'string' && value.length === 0);
      })
      .map(([key, type]) => `[${section}] ${key} (${type})`));
  if (bad.length > 0) {
    throw new Error(`${path} is missing, empty, or the wrong type for: ${bad.join(', ')}`);
  }

  // web-push rejects any other scheme, and the VAPID `sub` claim is what a
  // push service contacts if this server misbehaves.
  if (config.push) {
    const { subject } = config.push;
    if (!subject.startsWith('mailto:') && !subject.startsWith('https://')) {
      throw new Error(`[push] subject must start with mailto: or https://, not ${subject}`);
    }
  }

  // 0.0.0.0 means the server accepts on every interface, which is not an
  // address to connect to. Both processes run on one machine, so loopback
  // is where it is reachable.
  const host = config.bind.ip === '0.0.0.0' ? '127.0.0.1' : config.bind.ip;
  config.url = `ws://${host}:${config.bind.port}${SIDECAR_PATH}`;

  return config;
}
