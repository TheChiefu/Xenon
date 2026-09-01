// Refreshes stored tokens before Microsoft expires them, and removes the
// links that come back dead.
//
// Dead is `invalid_grant` on the refresh call, which is a changed password,
// revoked consent or a closed account, and a 401 with an XErr from the XSTS
// step, which is an account that became banned or ineligible. A 429, a 5xx,
// a network error and `invalid_client` all leave the link in place.

import * as store from './store.js';
import * as oauth from './oauth.js';
import { DeadLinkError, InvalidClientError } from './oauth.js';

const THIRTY_DAYS_MS = 30 * 24 * 60 * 60 * 1000;

/// Refreshes every credential last refreshed more than 30 days ago.
export async function refreshExpiring(config, send) {
  const cutoff = Date.now() - THIRTY_DAYS_MS;

  for (const account of store.all()) {
    if (account.lastRefreshedAt >= cutoff) continue;
    await refreshOne(account, config, send);
  }
}

async function refreshOne(account, config, send) {
  try {
    const tokens = await oauth.refreshAccessToken(account.refreshToken, config);
    if (tokens.refresh_token) {
      store.updateTokens(account.xuid, tokens.refresh_token);
    }
  } catch (e) {
    if (e instanceof DeadLinkError) {
      console.log(`${account.xuid}: link is dead (${e.message}), removing`);
      store.remove(account.xuid);
      send({ type: 'needs_reauth', user_id: account.userId, platform: 'xbox' });
      return;
    }
    if (e instanceof InvalidClientError) {
      console.error(`refresh failed for every account: our own client_secret is wrong (${e.message})`);
      return;
    }
    // Leaving lastRefreshedAt alone is what makes the next run retry.
    console.error(`${account.xuid}: refresh failed, retrying on the next run: ${e.message}`);
  }
}
