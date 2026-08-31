# Push sidecar maintenance

## subscriptions.json

Every browser subscribed is stored here, keyed by endpoint. The sidecar
rewrites the whole file on every subscribe/unsubscribe and reads it back at
startup.

Unlike `vapid.key`, this file is disposable: if it's lost, every browser's
next attempt to receive a push fails silently, and no one is notified to
resubscribe until they do so on their own (e.g. by reopening the app). There
is no automatic recovery for this — it's a low-stakes gap accepted for
simplicity, not a bug.

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

Every entry in `subscriptions.json` becomes undeliverable, so every user has
to subscribe again. A push service answers a request signed by a different
key with `403`, which the sidecar treats as `Rejected` — deliberately *not*
self-pruned (see `deliver`'s handling of `Outcome::Gone` vs `Outcome::Rejected`
in `src/main.rs`), since `Rejected` is also what a single stale row looks
like day-to-day, and auto-deleting on it would be indistinguishable from
auto-deleting everything the moment the key rotates. That means this cleanup
step stays manual:

1. Stop the sidecar.
2. Delete `vapid.key`.
3. Start the sidecar and record the new public key it prints.
4. Serve that key to browsers.
5. Clear `subscriptions.json` (or delete it — the sidecar recreates it as
   needed), so clients subscribe again rather than retrying rows that can
   never be delivered.
