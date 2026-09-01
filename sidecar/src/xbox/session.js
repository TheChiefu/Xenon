// Ties the credential store to the OAuth chain: getting a usable XSTS
// token for a linked account, refreshing lazily on expiry (or on a 401),
// never preemptively on a timer.

import * as store from './store.js';
import * as oauth from './oauth.js';

/// Returns a valid `{ token, uhs }` for `xuid`, refreshing the chain
/// (access token -> XSTS) if the cached one is missing or expired.
/// Persists a rotated refresh token immediately, before the access token
/// is used for anything. Persisting late costs exactly one successful
/// renewal, then permanent failure.
export async function ensureXsts(xuid, config) {
  const cached = store.validXsts(xuid);
  if (cached) return cached;

  const account = store.get(xuid);
  if (!account) throw new Error(`ensureXsts called for unlinked xuid ${xuid}`);

  const tokens = await oauth.refreshAccessToken(account.refreshToken, config);
  if (tokens.refresh_token) {
    store.updateTokens(xuid, tokens.refresh_token);
  }

  const xsts = await oauth.getXsts(tokens.access_token);
  store.cacheXsts(xuid, xsts.token, xsts.uhs, xsts.notAfter);
  return xsts;
}
