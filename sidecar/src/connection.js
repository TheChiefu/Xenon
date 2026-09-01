// The sidecar's WebSocket connection to Xenon, carrying push and Xbox
// messages both directions. The sidecar opens it.

import WebSocket from 'ws';

const RECONNECT_MS = 5000;

// Twelve attempts at five seconds is one minute of failing to connect.
const MAX_FAILURES = 12;

/// Connects to Xenon and reconnects 5 seconds after the connection closes.
/// Exits the process with status 1 after 12 attempts in a row fail before
/// opening.
///
/// - `config`: parsed config.toml, needs `url` and `sidecar.secret`.
/// - `onOpen(send)`: called once per successful connection, given a `send`
///   function for outgoing messages. Use it to report anything Xenon needs
///   to know right away (the VAPID key, a reconciliation request).
/// - `onMessage(type, msg, send)`: called for every incoming message.
export async function run(config, { onOpen, onMessage }) {
  let failures = 0;

  for (;;) {
    let opened = false;
    try {
      opened = await connectOnce(config, onOpen, onMessage);
    } catch (e) {
      console.error(`${config.url}: ${e.message}`);
    }

    // A connection that opened and later closed starts the count over, so
    // only an unreachable Xenon reaches the limit.
    failures = opened ? 0 : failures + 1;
    if (failures >= MAX_FAILURES) {
      console.error(`${config.url}: ${failures} attempts in a row failed, exiting`);
      process.exit(1);
    }

    await sleep(RECONNECT_MS);
  }
}

/// Runs one connection, resolving with whether it ever opened when it
/// closes, and rejecting if it fails before opening.
function connectOnce(config, onOpen, onMessage) {
  return new Promise((resolve, reject) => {
    let opened = false;

    const ws = new WebSocket(config.url, {
      headers: { Authorization: `Bearer ${config.sidecar.secret}` },
    });

    const send = (event) => ws.send(JSON.stringify(event));

    ws.on('open', () => {
      opened = true;
      console.log(`connected to ${config.url}`);
      onOpen(send);
    });

    ws.on('message', (data) => {
      let event;
      try {
        event = JSON.parse(data.toString());
      } catch (e) {
        console.error(`Xenon sent an event that did not parse: ${e.message}`);
        return;
      }

      const { type, ...rest } = event;
      onMessage(type, rest, send);
    });

    // A failure before the connection opens counts against MAX_FAILURES.
    ws.on('error', (e) => {
      if (opened) console.error(`${config.url}: ${e.message}`);
      else reject(e);
    });

    ws.on('close', () => resolve(opened));
  });
}

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}
