// How a game is shown, from what the service reports about it.

import { loadJson } from './atomicFile.js';

const FILE = 'games.json';

/// Removes the trademark and copyright marks from a game's name.
function removeMarks(name) {
  return name.replace(/[™®©]/g, '').replace(/ {2,}/g, ' ').trim();
}

/// The key a game is looked up on.
function lookupKey(name) {
  return name
    .normalize('NFC')
    .toLowerCase()
    .replace(/['’]/g, '') // Remove apostrophes
    .replace(/[^\p{L}\p{N}]+/gu, '_') // Remove non-alphanumeric characters
    .replace(/^_+|_+$/g, ''); // Remove leading and trailing underscores
}

/// Reads the list, returning an empty one when the file cannot be parsed.
function readList() {
  try {
    return loadJson(FILE);
  } catch (e) {
    console.error(`${FILE} ignored: ${e.message}`);
    return {};
  }
}

// Matched name -> what to show for that game.
const listed = new Map(
  Object.entries(readList()).map(([name, shown]) => [lookupKey(name), shown]),
);

/// The name to show for a game.
export function clean(name) {
  return listed.get(lookupKey(name))?.name || removeMarks(name);
}

/// The renamed prefix of a rich presence, and the rest in parentheses.
export function cleanActivity(name, activity) {
  const prefixes = listed.get(lookupKey(name))?.activity;
  if (!prefixes || !activity) return activity;

  for (const [prefix, shown] of Object.entries(prefixes)) {
    if (!activity.startsWith(prefix)) continue;

    const rest = activity.slice(prefix.length).replace(/^(: | - )/, '');
    return rest ? `${shown} (${rest})` : shown;
  }

  return activity;
}
