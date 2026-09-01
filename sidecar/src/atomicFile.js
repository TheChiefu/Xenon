// Shared JSON file persistence for both stores (push subscriptions, Xbox
// credentials): temp-write-then-rename for crash safety, mode 0600 because
// this machine runs other services (Caddy, a proxy) under separate
// accounts. 0600 makes this file readable only by the account running the
// sidecar, blocking those other accounts even if one were compromised. No
// encryption on top of that: 0600 already covers the threat identified,
// and decrypting by hand later for debugging would need its own tooling
// with no matching benefit here.

import { readFileSync, writeFileSync, renameSync } from 'node:fs';

/// Loads a JSON object from `path`, or `{}` if the file doesn't exist yet.
export function loadJson(path) {
  try {
    return JSON.parse(readFileSync(path, 'utf8'));
  } catch (e) {
    if (e.code === 'ENOENT') return {};
    throw e;
  }
}

/// Writes `data` to `path` atomically, at mode 0600.
export function saveJson(path, data) {
  const tmp = `${path}.tmp`;
  writeFileSync(tmp, JSON.stringify(data), { mode: 0o600 });
  renameSync(tmp, path);
}
