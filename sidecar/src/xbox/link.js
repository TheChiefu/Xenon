// The linking flow: Xenon says who wants to link and later hands over the
// callback's query parameters, this sidecar runs the exchange and reports
// the result.

import { randomBytes } from 'node:crypto';

import * as store from './store.js';
import { XuidTakenError } from './store.js';
import * as oauth from './oauth.js';
import { DeadLinkError, InvalidClientError } from './oauth.js';

const PLATFORM = 'xbox';
const AUTHORIZE_URL = 'https://login.live.com/oauth20_authorize.srf';
const SCOPE = 'XboxLive.signin XboxLive.offline_access';

/// Records which account is linking and answers with the address to sign in
/// at. The `state` in that address is what identifies the attempt when the
/// callback arrives.
///
/// - `event`: `{ user_id }`.
/// - `config`: parsed config.toml.
/// - `send`: sends one message to Xenon.
export function onLinkRequested(event, config, send) {
  const state = randomBytes(16).toString('hex');
  store.registerPending(state, event.user_id);

  const url = new URL(AUTHORIZE_URL);
  url.search = new URLSearchParams({
    client_id: config.xbox.client_id,
    response_type: 'code',
    redirect_uri: config.xbox.redirect_uri,
    scope: SCOPE,
    state,
  });

  console.log(`${event.user_id}: sign in at ${url}`);

  send({
    type: 'link_url',
    user_id: event.user_id,
    platform: PLATFORM,
    authorize_url: url.toString(),
  });
}

/// Runs the token exchange for one callback and reports the result. The
/// account linked is the one recorded against this state, not one named in
/// the parameters.
///
/// - `event`: `{ params }`, the callback's query string as key/value pairs.
/// - `config`: parsed config.toml.
/// - `send`: sends one message to Xenon.
export async function onLinkCallback(event, config, send) {
  const { state, code, error } = event.params ?? {};
  const userId = store.takePending(state);

  if (!userId) {
    console.error(`a callback arrived for an unknown state: ${state}`);
    return;
  }

  if (error) {
    fail(send, userId, error);
    return;
  }

  try {
    const tokens = await oauth.exchangeCode(code, config);
    const xsts = await oauth.getXsts(tokens.access_token);

    store.upsert(userId, xsts.xuid, xsts.gamertag, tokens.refresh_token);
    store.cacheXsts(xsts.xuid, xsts.token, xsts.uhs, xsts.notAfter);

    send({
      type: 'link_result',
      outcome: {
        status: 'linked',
        user_id: userId,
        platform: PLATFORM,
        handle: xsts.gamertag,
      },
    });
  } catch (e) {
    if (e instanceof XuidTakenError) {
      fail(send, userId, 'this Xbox account is already linked to another user');
    } else if (e instanceof DeadLinkError) {
      fail(send, userId, e.message);
    } else if (e instanceof InvalidClientError) {
      console.error(`xbox link failed, our own client_secret is wrong: ${e.message}`);
      fail(send, userId, 'linking is temporarily unavailable');
    } else {
      console.error(`xbox link exchange failed: ${e.message}`);
      fail(send, userId, 'something went wrong');
    }
  }
}

function fail(send, userId, message) {
  send({
    type: 'link_result',
    outcome: { status: 'error', user_id: userId, platform: PLATFORM, message },
  });
}
