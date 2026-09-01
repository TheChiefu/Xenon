// Browser subscriptions, in memory and mirrored to subscriptions.json. This
// process is the only writer, one message at a time, so every change
// rewrites the whole file.

import { loadJson, saveJson } from '../atomicFile.js';

const FILE = 'subscriptions.json';

let byEndpoint = loadJson(FILE); // endpoint -> { userId, p256dh, auth } (p256dh/auth as base64url strings)
let byUser = new Map(); // userId -> Set<endpoint>, rebuilt below on each start

for (const [endpoint, entry] of Object.entries(byEndpoint)) {
  addToUserIndex(entry.userId, endpoint);
}

/// Stores a browser's subscription, replacing any earlier owner of the
/// same endpoint.
///
/// - `userId`: account the browser is signed in as.
/// - `subscription`: `{ endpoint, p256dh, auth }`, keys as base64url strings.
export function upsert(userId, subscription) {
  const { endpoint, p256dh, auth } = subscription;
  const previous = byEndpoint[endpoint];
  if (previous && previous.userId !== userId) {
    byUser.get(previous.userId)?.delete(endpoint);
  }

  byEndpoint[endpoint] = { userId, p256dh, auth };
  addToUserIndex(userId, endpoint);
  save();
}

/// Removes one browser's subscription, if it belongs to `userId`.
export function remove(userId, endpoint) {
  if (byEndpoint[endpoint]?.userId === userId) {
    removeByEndpoint(endpoint);
  }
}

/// Removes one browser's subscription regardless of owner, for an endpoint
/// a push service answered 404 or 410 for.
export function removeByEndpoint(endpoint) {
  const entry = byEndpoint[endpoint];
  if (!entry) return;

  delete byEndpoint[endpoint];
  byUser.get(entry.userId)?.delete(endpoint);
  save();
}

/// Reads every subscription belonging to any of `userIds`.
export function subscriptionsFor(userIds) {
  const subscriptions = [];
  for (const userId of userIds) {
    for (const endpoint of byUser.get(userId) ?? []) {
      const entry = byEndpoint[endpoint];
      if (entry) subscriptions.push({ endpoint, p256dh: entry.p256dh, auth: entry.auth });
    }
  }
  return subscriptions;
}

function addToUserIndex(userId, endpoint) {
  if (!byUser.has(userId)) byUser.set(userId, new Set());
  byUser.get(userId).add(endpoint);
}

function save() {
  saveJson(FILE, byEndpoint);
}
