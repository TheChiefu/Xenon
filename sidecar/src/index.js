import { readConfig } from './config.js';
import { run } from './connection.js';
import * as push from './push/index.js';
import * as xboxStore from './xbox/store.js';
import * as xboxLink from './xbox/link.js';
import * as xboxPoll from './xbox/poll.js';
import * as xboxRefresh from './xbox/refresh.js';

// A bad config is something the operator has to go fix, so print the one
// line naming what is wrong. A throw after this point prints its stack.
let config;
try {
  config = readConfig();
} catch (e) {
  console.error(e.message);
  process.exit(1);
}

// A service runs when its section is in config.toml. readConfig has already
// refused a file with neither, so at least one of these is true.
const pushEnabled = config.push !== undefined;
const xboxEnabled = config.xbox !== undefined;
console.log(`running: ${[pushEnabled && 'push', xboxEnabled && 'xbox'].filter(Boolean).join(', ')}`);

if (pushEnabled) push.init(config.push.subject);

// The active connection's send function, used by the timers below. They run
// on their own schedule, independent of any one connection attempt, and do
// nothing while disconnected.
let currentSend = () => {};

if (xboxEnabled) {
  setInterval(() => xboxStore.dropStalePending(), 60_000);
  setInterval(() => xboxRefresh.refreshExpiring(config, currentSend).catch((e) => console.error(`token refresh failed: ${e.message}`)), config.xbox.refresh_interval_seconds * 1000);
  setInterval(() => xboxPoll.tick(config, currentSend).catch((e) => console.error(`poll tick failed: ${e.message}`)), config.xbox.poll_interval_seconds * 1000);
}

run(config, {
  onOpen(send) {
    currentSend = send;
    // Without a VAPID key Xenon has nothing to serve at GET /push/vapid, so
    // no browser can subscribe and no job is ever composed.
    if (pushEnabled) send({ type: 'vapid_key', public_key: push.publicKeyBytes() });
    if (xboxEnabled) {
      send({ type: 'get_linked_accounts' });
      // Xenon may have just started, holding no presence at all
      xboxPoll.clearCache();
    }
  },

  onMessage(type, event, send) {
    switch (type) {
      case 'push':
        if (!pushEnabled) return unconfigured(type);
        push.deliver(event, config.push.ttl).catch((e) => console.error(`push delivery failed: ${e.message}`));
        break;
      case 'subscribe':
        if (!pushEnabled) return unconfigured(type);
        push.subscribe(event.user_id, event.subscription);
        break;
      case 'unsubscribe':
        if (!pushEnabled) return unconfigured(type);
        push.unsubscribe(event.user_id, event.endpoint);
        break;
      case 'link_requested':
        if (!xboxEnabled) return unconfigured(type);
        xboxLink.onLinkRequested(event, config, send);
        break;
      case 'link_callback':
        if (!xboxEnabled) return unconfigured(type);
        xboxLink.onLinkCallback(event, config, send).catch((e) => console.error(`callback handling failed: ${e.message}`));
        break;
      case 'linked_accounts':
        if (!xboxEnabled) return unconfigured(type);
        xboxStore.reconcile(event.user_ids);
        break;
      default:
        console.error(`Xenon sent an event of unknown type: ${type}`);
    }
  },
});

/// Reports an event for a service this sidecar was not configured to run.
/// Xenon only sends these when its own config and this one disagree.
function unconfigured(type) {
  console.error(`Xenon sent ${type}, which this sidecar is not configured for`);
}
