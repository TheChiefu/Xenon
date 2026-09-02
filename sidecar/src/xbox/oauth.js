// The Microsoft and Xbox Live OAuth exchanges. HTTP and JSON throughout:
// Microsoft does the signing.
// Docs: https://github.com/MicrosoftDocs/xbox-live-docs/tree/docs

const AUTHORIZE_TOKEN_URL = 'https://login.live.com/oauth20_token.srf';
const USER_AUTH_URL = 'https://user.auth.xboxlive.com/user/authenticate';
const XSTS_AUTH_URL = 'https://xsts.auth.xboxlive.com/xsts/authorize';

/// Thrown for a failure that only re-linking fixes: `invalid_grant` on
/// refresh, or a 401 with an XErr from the XSTS step.
export class DeadLinkError extends Error {}

/// Thrown when the sidecar's own client_secret is wrong or expired.
/// Affects every user identically; their accounts are fine.
export class InvalidClientError extends Error {}

/// Exchanges an authorization code for tokens, during the initial link.
export async function exchangeCode(code, config) {
  const body = new URLSearchParams({
    client_id: config.xbox.client_id,
    client_secret: config.xbox.client_secret,
    code,
    grant_type: 'authorization_code',
    redirect_uri: config.xbox.redirect_uri,
    scope: 'XboxLive.signin XboxLive.offline_access',
  });
  return tokenRequest(body);
}

/// Trades a stored refresh token for a fresh access token. The response
/// contains a new refresh token every time, and the caller must store it
/// before using the access token, or a later start reads an invalidated one.
export async function refreshAccessToken(refreshToken, config) {
  const body = new URLSearchParams({
    client_id: config.xbox.client_id,
    client_secret: config.xbox.client_secret,
    refresh_token: refreshToken,
    grant_type: 'refresh_token',
    scope: 'XboxLive.signin XboxLive.offline_access',
  });
  return tokenRequest(body);
}

async function tokenRequest(body) {
  const response = await fetch(AUTHORIZE_TOKEN_URL, {
    method: 'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body,
  });

  const json = await response.json();

  if (!response.ok) {
    if (json.error === 'invalid_grant') throw new DeadLinkError(json.error_description ?? 'invalid_grant');
    if (json.error === 'invalid_client') throw new InvalidClientError(json.error_description ?? 'invalid_client');
    throw new Error(`token request failed: ${json.error ?? response.status}`);
  }

  return json;
}

const XERR_MESSAGES = {
  2148916227: 'account banned',
  2148916233: 'no Xbox profile on this Microsoft account',
  2148916235: 'Xbox Live unavailable in this region',
  2148916236: 'adult verification required',
  2148916237: 'adult verification required',
  2148916238: 'child account not in a family group',
};

/// Converts a Microsoft access token into an XSTS token, XUID and gamertag.
/// Throws `DeadLinkError` for any XErr.
export async function getXsts(accessToken) {
  const userToken = await userAuthenticate(accessToken);

  const response = await fetch(XSTS_AUTH_URL, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'x-xbl-contract-version': '1' },
    body: JSON.stringify({
      RelyingParty: 'http://xboxlive.com',
      TokenType: 'JWT',
      Properties: { SandboxId: 'RETAIL', UserTokens: [userToken] },
    }),
  });

  const json = await response.json();

  if (!response.ok) {
    const message = XERR_MESSAGES[json.XErr] ?? `XErr ${json.XErr}`;
    throw new DeadLinkError(message);
  }

  const claims = json.DisplayClaims.xui[0];
  return {
    xuid: claims.xid,
    gamertag: claims.gtg,
    uhs: claims.uhs,
    token: json.Token,
    notAfter: Date.parse(json.NotAfter),
  };
}

async function userAuthenticate(accessToken) {
  const response = await fetch(USER_AUTH_URL, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'x-xbl-contract-version': '1' },
    body: JSON.stringify({
      RelyingParty: 'http://auth.xboxlive.com',
      TokenType: 'JWT',
      Properties: { AuthMethod: 'RPS', SiteName: 'user.auth.xboxlive.com', RpsTicket: `d=${accessToken}` },
    }),
  });

  if (!response.ok) throw new Error(`user authenticate failed: ${response.status}`);
  const json = await response.json();
  return json.Token;
}
