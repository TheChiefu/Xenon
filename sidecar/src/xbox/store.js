// Xbox credentials: refresh tokens, the XUID mapping, and the rule that one
// XUID belongs to one user. Pending link attempts and the XSTS token cache
// are in memory only, rebuilt after a restart.

import { loadJson, saveJson } from '../atomicFile.js';

const FILE = 'xbox_links.json';
const PENDING_TTL_MS = 10 * 60 * 1000;

let byXuid = loadJson(FILE); // xuid -> { userId, gamertag, refreshToken, lastRefreshedAt }
let byUser = new Map(); // userId -> xuid, rebuilt below on each start
const pending = new Map(); // state -> { userId, createdAt }
const xsts = new Map(); // xuid -> { token, uhs, notAfter }

for (const [xuid, entry] of Object.entries(byXuid)) {
  byUser.set(entry.userId, xuid);
}

/// Records a pending link attempt, keyed by the `state` the browser will
/// come back with.
export function registerPending(state, userId) {
  pending.set(state, { userId, createdAt: Date.now() });
}

/// Looks up and consumes a pending attempt, returning `undefined` when the
/// state is unknown or its attempt is older than 10 minutes.
export function takePending(state) {
  const entry = pending.get(state);
  pending.delete(state);
  if (!entry || Date.now() - entry.createdAt > PENDING_TTL_MS) return undefined;
  return entry.userId;
}

/// Drops pending attempts older than 10 minutes.
export function dropStalePending() {
  const cutoff = Date.now() - PENDING_TTL_MS;
  for (const [state, entry] of pending) {
    if (entry.createdAt < cutoff) pending.delete(state);
  }
}

/// Thrown by `upsert` when `xuid` already belongs to a different user.
export class XuidTakenError extends Error {}

/// Stores a linked account, replacing whatever XUID this user had. Throws
/// `XuidTakenError` when the XUID belongs to someone else.
export function upsert(userId, xuid, gamertag, refreshToken) {
  const existing = byXuid[xuid];
  if (existing && existing.userId !== userId) {
    throw new XuidTakenError(`xuid ${xuid} is already linked to another user`);
  }

  const previousXuid = byUser.get(userId);
  if (previousXuid && previousXuid !== xuid) {
    delete byXuid[previousXuid];
  }

  byXuid[xuid] = { userId, gamertag, refreshToken, lastRefreshedAt: Date.now() };
  byUser.set(userId, xuid);
  save();
}

/// Replaces the refresh token of a linked account, throwing when the XUID
/// has no entry.
export function updateTokens(xuid, refreshToken) {
  const entry = byXuid[xuid];
  if (!entry) throw new Error(`updateTokens called for unknown xuid ${xuid}`);
  entry.refreshToken = refreshToken;
  entry.lastRefreshedAt = Date.now();
  save();
}

/// Removes one XUID's stored credentials and its cached XSTS token.
export function remove(xuid) {
  const entry = byXuid[xuid];
  if (!entry) return;
  delete byXuid[xuid];
  byUser.delete(entry.userId);
  xsts.delete(xuid);
  save();
}

export function get(xuid) {
  return byXuid[xuid];
}

export function all() {
  return Object.entries(byXuid).map(([xuid, entry]) => ({ xuid, ...entry }));
}

/// Drops every stored credential whose user is missing from `userIds`.
export function reconcile(userIds) {
  const keep = new Set(userIds);
  for (const { xuid, userId } of all()) {
    if (!keep.has(userId)) remove(xuid);
  }
}

export function cacheXsts(xuid, token, uhs, notAfter) {
  xsts.set(xuid, { token, uhs, notAfter });
}

/// Returns the cached XSTS token if it's still valid, else `undefined`.
export function validXsts(xuid) {
  const entry = xsts.get(xuid);
  if (!entry || Date.now() >= entry.notAfter) return undefined;
  return entry;
}

function save() {
  saveJson(FILE, byXuid);
}
