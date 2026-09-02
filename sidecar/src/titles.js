// What a game is called here, from the name the service reports it under.
// One list for every platform: services name the same game the same way.

import { loadJson } from './atomicFile.js';

const FILE = 'game_titles.json';

/// Removes the trademark and copyright marks from a game's name.
function removeMarks(name) {
  return name.replace(/[™®©]/g, '').replace(/ {2,}/g, ' ').trim();
}

/// The form a name is matched in. Apostrophes are dropped, and every run of
/// spaces, marks and punctuation becomes one underscore.
function lookupKey(name) {
  return name
    .normalize('NFC')
    .toLowerCase()
    .replace(/['’]/g, '')
    .replace(/[^\p{L}\p{N}]+/gu, '_')
    .replace(/^_+|_+$/g, '');
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

// Matched name -> name to show, hand-written, absent until someone writes it.
const named = new Map(
  Object.entries(readList()).map(([name, shown]) => [lookupKey(name), shown]),
);

/// Strips the trademark and copyright marks from a game's name and returns the
/// name to show. A name the list has no entry for is shown stripped.
export function clean(name) {
  return named.get(lookupKey(name)) || removeMarks(name);
}
