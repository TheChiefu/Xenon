// Presence polling: one call per linked account per tick, each made with
// that account's own token, since Xbox gates reading someone else's
// presence behind its friendship and privacy settings.
//
// Xbox Live allows a 15-second burst window and a 5-minute sustain window
// per user per title, and each linked user spends its own budget. That is
// the limit config.xbox.poll_interval_seconds sits under.

import * as store from './store.js';
import { ensureXsts } from './session.js';
import { DeadLinkError, InvalidClientError } from './oauth.js';
import { clean } from '../titles.js';

const presenceCache = new Map(); // xuid -> last-known { status, title, activity }

/// Forgets what every account last reported, so the next tick sends its
/// presence again rather than returning early on an unchanged one.
///
/// Called when the connection to Xenon opens. Xenon holds what it was last
/// told, and a Xenon that just started holds nothing, so the cache here has to
/// stop matching for that to be filled in again.
export function clearCache() {
  presenceCache.clear();
}

export async function tick(config, send) {
  for (const account of store.all()) {
    await pollOne(account, config, send);
  }
}

async function pollOne(account, config, send) {
  let xsts;
  try {
    xsts = await ensureXsts(account.xuid, config);
  } catch (e) {
    if (e instanceof DeadLinkError) {
      console.log(`${account.xuid}: link is dead (${e.message}), removing`);
      store.remove(account.xuid);
      send({ type: 'needs_reauth', user_id: account.userId, platform: 'xbox' });
    } else if (e instanceof InvalidClientError) {
      console.error(`presence poll failed for every account: our own client_secret is wrong (${e.message})`);
    } else {
      console.error(`${account.xuid}: could not get a session, skipping this tick: ${e.message}`);
    }
    return;
  }

  let presence;
  try {
    presence = await fetchPresence(account.xuid, xsts);
  } catch (e) {
    console.error(`${account.xuid}: presence poll failed, skipping this tick: ${e.message}`);
    return;
  }

  const previous = presenceCache.get(account.xuid);
  if (previous
    && previous.status === presence.status
    && previous.title === presence.title
    && previous.activity === presence.activity) return;

  presenceCache.set(account.xuid, presence);
  send({
    type: 'presence',
    user_id: account.userId,
    platform: 'xbox',
    status: presence.status,
    title: presence.title,
    activity: presence.activity,
  });
}

// https://learn.microsoft.com/en-us/gaming/gdk/docs/reference/live/rest/uri/presence/uri-usersxuidget
//
// `level` selects how much of the record comes back. The default, `title`,
// returns everything except each title's activity, so `all` is what makes
// `richPresence` present.
async function fetchPresence(xuid, xsts) {
  const response = await fetch(`https://userpresence.xboxlive.com/users/xuid(${xuid})?level=all`, {
    headers: {
      Authorization: `XBL3.0 x=${xsts.uhs};${xsts.token}`,
      'x-xbl-contract-version': '3',
      Accept: 'application/json',
      'Accept-Language': 'en-US',
    },
  });

  if (!response.ok) throw new Error(`presence request failed: ${response.status}`);
  const json = await response.json();
  return parsePresence(json);
}

// https://learn.microsoft.com/en-us/gaming/gdk/docs/reference/live/rest/json/json-presencerecord
//
/// Reads a presence response. `state` selects which of `lastSeen` and
/// `devices` the body contains, and is reported in Xbox's own words.
function parsePresence(json) {
  if (json.state !== 'Online') {
    return { status: json.state, title: undefined, activity: undefined };
  }

  // A user can appear under several devices at once, a console and a phone.
  // Full placement is the title in the foreground; the others are the Xbox
  // shell, Home and the mobile app.
  //
  // `name` is the game. `richPresence` describes what they are doing inside
  // it, such as "Main Menu - Title Screen", and many titles leave it unset.
  // They are sent apart, so Xenon decides how the two read together.
  for (const device of json.devices ?? []) {
    for (const title of device.titles ?? []) {
      if (title.placement !== 'Full') continue;

      return {
        status: json.state,
        title: clean(title.name),
        activity: title.activity?.richPresence,
      };
    }
  }

  return { status: json.state, title: undefined, activity: undefined };
}
