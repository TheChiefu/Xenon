// VAPID signing, RFC 8291 encryption, and delivery, all through the
// web-push package.

import webpush from 'web-push';
import { loadJson, saveJson } from '../atomicFile.js';
import * as store from './store.js';

const KEY_FILE = 'vapid.json';

let keys;

/// Loads the VAPID keypair, generating one on first run. Every subscription
/// is bound to the public key it was created with, so replacing the keypair
/// orphans all of them silently.
export function init(subject) {
  keys = loadJson(KEY_FILE);
  if (!keys.publicKey) {
    keys = webpush.generateVAPIDKeys();
    saveJson(KEY_FILE, keys);
  }
  webpush.setVapidDetails(subject, keys.publicKey, keys.privateKey);
}

/// The public key as a byte array.
export function publicKeyBytes() {
  return Array.from(Buffer.from(keys.publicKey, 'base64url'));
}

/// Registers a browser's subscription.
///
/// - `userId`: account the browser is signed in as.
/// - `subscription`: `{ endpoint, p256dh, auth }`, the two keys as byte
///   arrays.
export function subscribe(userId, subscription) {
  store.upsert(userId, {
    endpoint: subscription.endpoint,
    p256dh: bytesToBase64Url(subscription.p256dh),
    auth: bytesToBase64Url(subscription.auth),
  });
}

/// Removes one of the caller's subscriptions.
export function unsubscribe(userId, endpoint) {
  store.remove(userId, endpoint);
}

/// Sends one notification to every browser subscribed for the named users.
///
/// - `job`: `{ room_id, room_name, author, body, renotify, user_ids }`.
/// - `ttl`: seconds a push service stores the message for a device that is
///   switched off.
export async function deliver(job, ttl) {
  const subscriptions = store.subscriptionsFor(job.user_ids);
  if (subscriptions.length === 0) return;

  const payload = JSON.stringify({
    room_id: job.room_id,
    room: job.room_name,
    author: job.author,
    body: job.body,
    renotify: job.renotify,
  });

  for (const subscription of subscriptions) {
    const target = {
      endpoint: subscription.endpoint,
      keys: { p256dh: subscription.p256dh, auth: subscription.auth },
    };

    try {
      await webpush.sendNotification(target, payload, { TTL: ttl });
      console.log(`${subscription.endpoint}: accepted`);
    } catch (e) {
      handleFailure(subscription.endpoint, e);
    }
  }
}

function handleFailure(endpoint, e) {
  switch (e.statusCode) {
    case 404:
    case 410:
      // The push service confirms the browser will never accept this
      // endpoint again, so it's safe to forget.
      console.log(`${endpoint}: the subscription no longer exists, removing`);
      store.removeByEndpoint(endpoint);
      break;
    case 403:
      // A replaced VAPID key answers 403 for every subscription at once, so
      // this one is logged and kept rather than removed.
      console.error(`${endpoint}: signed with the wrong key`);
      break;
    case 413:
      console.error(`${endpoint}: the payload is over the service's limit`);
      break;
    case 429: {
      const retryAfter = e.headers?.['retry-after'];
      console.error(`${endpoint}: sending too fast${retryAfter ? `, retry after ${retryAfter}s` : ''}`);
      break;
    }
    default:
      console.error(`${endpoint}: ${e.statusCode ? `answered ${e.statusCode}` : e.message}`);
  }
}

function bytesToBase64Url(bytes) {
  return Buffer.from(bytes).toString('base64url');
}
