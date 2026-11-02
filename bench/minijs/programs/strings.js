// Strings: string building and manipulation. Builds text by repeated `+=`
// concatenation, number-to-string conversion and join; then splits it,
// counts words in an object used as a dictionary, run-length encodes and
// decodes it, and formats a padded table. Checksums use charCodeAt.
const ROUNDS = 190;
const WORDS_PER_ROUND = 4000;

let seed = 12345;
function rand() {
  seed = (seed * 16807) % 2147483647;
  return seed;
}

const SYLLABLES = ["ka", "lo", "mi", "ne", "su", "ta", "vo", "ri", "ex", "on", "aa", "zz"];

function makeWord() {
  const n = 1 + (rand() >> 8) % 3;
  let w = "";
  for (let i = 0; i < n; i++) w += SYLLABLES[(rand() >> 8) % SYLLABLES.length];
  return w;
}

function hashString(s) {
  let h = 5381;
  for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) | 0;
  return h;
}

function buildText(nWords) {
  let text = "";
  for (let i = 0; i < nWords; i++) {
    if (i > 0) text += (i % 13 === 0) ? ".\n" : " ";
    text += makeWord();
    if (i % 17 === 0) text += String(i * 7 % 1000);
  }
  return text;
}

function runLengthEncode(s) {
  const parts = [];
  let i = 0;
  while (i < s.length) {
    const c = s[i];
    let j = i + 1;
    while (j < s.length && s[j] === c) j++;
    const n = j - i;
    parts.push(n > 1 ? n + ":" + c : ":" + c);
    i = j;
  }
  return parts.join("");
}

function runLengthDecode(s) {
  // Each run is "count:char" or ":char"; the char may itself be a digit.
  let out = "";
  let n = 0;
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    if (c === 58) {
      const ch = s.charAt(i + 1);
      out += n > 0 ? ch.repeat(n) : ch;
      n = 0;
      i++;
    } else {
      n = n * 10 + (c - 48);
    }
  }
  return out;
}

function pad(s, width, right) {
  if (s.length >= width) return s;
  const fill = " ".repeat(width - s.length);
  return right ? s + fill : fill + s;
}

function capitalize(w) {
  return w.length === 0 ? w : w.charAt(0).toUpperCase() + w.slice(1);
}

function reverse(s) {
  let r = "";
  for (let i = s.length - 1; i >= 0; i--) r += s[i];
  return r;
}

function round() {
  const text = buildText(WORDS_PER_ROUND);
  // Count words per sentence and globally.
  const sentences = text.split(".\n");
  const counts = {};
  const order = [];
  let wordTotal = 0;
  for (const sent of sentences) {
    const words = sent.trim().split(" ");
    for (const w of words) {
      if (w.length === 0) continue;
      wordTotal++;
      const key = "w_" + w;
      if (counts.hasOwnProperty(key)) {
        counts[key]++;
      } else {
        counts[key] = 1;
        order.push(w);
      }
    }
  }
  order.sort((a, b) => {
    const d = counts["w_" + b] - counts["w_" + a];
    if (d !== 0) return d;
    return a < b ? -1 : (a > b ? 1 : 0);
  });
  // Format a table of the top entries.
  let table = "";
  for (let i = 0; i < 40 && i < order.length; i++) {
    const w = order[i];
    table += pad(String(i + 1), 4, false) + ". " + pad(capitalize(w), 12, true) +
             pad(String(counts["w_" + w]), 6, false) + "\n";
  }
  // Upper/lower case, reverse, replace via split/join.
  const upper = text.toUpperCase();
  const swapped = upper.split("KA").join("ka-").toLowerCase();
  const rev = reverse(sentences[1]);
  // Run-length round trip on a text with long runs.
  const runs = text.slice(0, 2000).split("a").join("aaaa").split("z").join("zzzzzzzzzzzz");
  const enc = runLengthEncode(runs);
  const dec = runLengthDecode(enc);
  if (dec !== runs) throw new Error("rle mismatch");
  // Search.
  let found = 0;
  let pos = text.indexOf("kalo");
  while (pos >= 0) {
    found++;
    pos = text.indexOf("kalo", pos + 1);
  }
  return [hashString(text), hashString(table), hashString(swapped), hashString(rev),
          enc.length, wordTotal, order.length, found,
          text.lastIndexOf("mi"), table.length];
}

let check = 0;
let last = null;
for (let r = 0; r < ROUNDS; r++) {
  last = round();
  for (const v of last) check = (check * 31 + v) | 0;
}
console.log("strings", ROUNDS, check);
console.log("last", last.join(" "));
