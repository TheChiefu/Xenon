# Push sidecar maintenance

## vapid.key

A push service verifies which server each request came from, and VAPID defines
that check. The specifications call the sending server the application server,
which here is the sidecar. The application server stores a P-256 key pair. It
signs a token with the private half on every request, which the push service
verifies against the public half.

The public half is also what a browser receives when it subscribes, under the
name `applicationServerKey`.

`vapid.key` stores that pair. The sidecar generates it the first time it runs
and reads the same file on every run after that. The file is created at mode
`0600`, so only its owner can read it.

**Back this file up.** A subscription records the key it was created with, so
the pair cannot be regenerated without invalidating every subscription.

References: [RFC 8292](https://www.rfc-editor.org/rfc/rfc8292),
[Push API](https://www.w3.org/TR/push-api/)

## Replacing the key

Every row in `push_subscriptions` becomes undeliverable, so every user has to
subscribe again. A push service answers a request signed by a different key
with `403`.

1. Stop the sidecar.
2. Delete `vapid.key`.
3. Start the sidecar and record the new public key it prints.
4. Serve that key to browsers.
5. Clear `push_subscriptions`, so clients subscribe again rather than retrying
   rows that can never be delivered.
