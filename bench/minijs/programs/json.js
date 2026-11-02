// JSON: a hand-written JSON tokenizer, recursive-descent parser and
// serializer (JSON itself is not part of MiniJS). A deterministic generator
// builds a nested document as text; it is parsed, walked, serialized again
// and re-parsed. Stresses charCodeAt, string slicing and building, object
// creation with dynamic keys, and recursion.
const ROUNDS = 25;
const DOC_ITEMS = 1500;

// --- deterministic generator (Park-Miller minimal standard) ---
let seed = 42;
function rand() {
  seed = (seed * 16807) % 2147483647;
  return seed;
}
function randInt(n) {
  return rand() % n;
}

const WORDS = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta",
  "theta", "iota", "kappa", "lambda", "mu", "nu", "xi", "omicron", "pi"];

function genString() {
  let s = "\"";
  const n = 1 + randInt(3);
  for (let i = 0; i < n; i++) {
    if (i > 0) s += randInt(5) === 0 ? "\\n" : " ";
    s += WORDS[randInt(WORDS.length)];
  }
  if (randInt(7) === 0) s += "\\\"q\\\"";
  if (randInt(9) === 0) s += "\\\\";
  return s + "\"";
}

function genNumber() {
  const k = randInt(4);
  if (k === 0) return String(randInt(1000));
  if (k === 1) return "-" + String(randInt(100000));
  if (k === 2) return String(randInt(1000)) + "." + String(randInt(1000));
  return String(randInt(90) + 1) + "e" + String(randInt(5));
}

function genValue(depth) {
  const k = randInt(depth > 3 ? 6 : 9);
  switch (k) {
    case 0: case 1: return genNumber();
    case 2: case 3: return genString();
    case 4: return randInt(2) === 0 ? "true" : "false";
    case 5: return "null";
    case 6: case 7: {
      const n = randInt(6);
      const parts = [];
      for (let i = 0; i < n; i++) {
        parts.push("\"" + WORDS[randInt(WORDS.length)] + i + "\" : " + genValue(depth + 1));
      }
      return "{ " + parts.join(",\n  ") + " }";
    }
    default: {
      const n = randInt(7);
      const parts = [];
      for (let i = 0; i < n; i++) parts.push(genValue(depth + 1));
      return "[" + parts.join(", ") + "]";
    }
  }
}

function genDocument(items) {
  const parts = [];
  for (let i = 0; i < items; i++) {
    parts.push("{\"id\": " + i + ", \"name\": " + genString() +
               ", \"payload\": " + genValue(0) + "}");
  }
  return "{\"version\": 3, \"items\": [\n" + parts.join(",\n") + "\n]}";
}

// --- tokenizer ---
const T_LBRACE = 1;
const T_RBRACE = 2;
const T_LBRACK = 3;
const T_RBRACK = 4;
const T_COLON = 5;
const T_COMMA = 6;
const T_STRING = 7;
const T_NUMBER = 8;
const T_TRUE = 9;
const T_FALSE = 10;
const T_NULL = 11;
const T_EOF = 12;

class Lexer {
  constructor(text) {
    this.text = text;
    this.pos = 0;
    this.kind = 0;
    this.value = null;
  }
  error(msg) {
    throw new Error("lex error at " + this.pos + ": " + msg);
  }
  skipSpace() {
    const t = this.text;
    while (this.pos < t.length) {
      const c = t.charCodeAt(this.pos);
      if (c === 32 || c === 10 || c === 13 || c === 9) this.pos++;
      else break;
    }
  }
  next() {
    this.skipSpace();
    const t = this.text;
    if (this.pos >= t.length) {
      this.kind = T_EOF;
      return;
    }
    const c = t.charCodeAt(this.pos);
    switch (c) {
      case 123: this.pos++; this.kind = T_LBRACE; return;
      case 125: this.pos++; this.kind = T_RBRACE; return;
      case 91: this.pos++; this.kind = T_LBRACK; return;
      case 93: this.pos++; this.kind = T_RBRACK; return;
      case 58: this.pos++; this.kind = T_COLON; return;
      case 44: this.pos++; this.kind = T_COMMA; return;
      case 34: this.lexString(); return;
      case 116: this.lexWord("true", T_TRUE); return;
      case 102: this.lexWord("false", T_FALSE); return;
      case 110: this.lexWord("null", T_NULL); return;
    }
    if (c === 45 || (c >= 48 && c <= 57)) {
      this.lexNumber();
      return;
    }
    this.error("unexpected character " + t.charAt(this.pos));
  }
  lexWord(word, kind) {
    if (this.text.slice(this.pos, this.pos + word.length) !== word) this.error("bad word");
    this.pos += word.length;
    this.kind = kind;
  }
  lexString() {
    const t = this.text;
    this.pos++;
    let out = "";
    let start = this.pos;
    for (;;) {
      if (this.pos >= t.length) this.error("unterminated string");
      const c = t.charCodeAt(this.pos);
      if (c === 34) {
        out += t.slice(start, this.pos);
        this.pos++;
        break;
      }
      if (c === 92) {
        out += t.slice(start, this.pos);
        const e = t.charAt(this.pos + 1);
        if (e === "n") out += "\n";
        else if (e === "t") out += "\t";
        else if (e === "\"") out += "\"";
        else if (e === "\\") out += "\\";
        else if (e === "/") out += "/";
        else this.error("bad escape");
        this.pos += 2;
        start = this.pos;
      } else {
        this.pos++;
      }
    }
    this.kind = T_STRING;
    this.value = out;
  }
  lexNumber() {
    const t = this.text;
    const start = this.pos;
    let neg = false;
    if (t.charCodeAt(this.pos) === 45) {
      neg = true;
      this.pos++;
    }
    let intPart = 0;
    let c = t.charCodeAt(this.pos);
    while (c >= 48 && c <= 57) {
      intPart = intPart * 10 + (c - 48);
      this.pos++;
      c = t.charCodeAt(this.pos);
    }
    if (c === 46 || c === 101 || c === 69) {
      // Fraction or exponent: collect the rest and let Number() convert.
      this.pos++;
      c = t.charCodeAt(this.pos);
      while ((c >= 48 && c <= 57) || c === 45 || c === 43 || c === 101 || c === 69) {
        this.pos++;
        c = t.charCodeAt(this.pos);
      }
      this.value = Number(t.slice(start, this.pos));
    } else {
      this.value = neg ? -intPart : intPart;
    }
    this.kind = T_NUMBER;
  }
}

// --- parser ---
class Parser {
  constructor(text) {
    this.lex = new Lexer(text);
    this.lex.next();
  }
  expect(kind) {
    if (this.lex.kind !== kind) throw new Error("parse error: expected " + kind + " got " + this.lex.kind);
    this.lex.next();
  }
  parseValue() {
    const lex = this.lex;
    switch (lex.kind) {
      case T_LBRACE: return this.parseObject();
      case T_LBRACK: return this.parseArray();
      case T_STRING:
      case T_NUMBER: {
        const v = lex.value;
        lex.next();
        return v;
      }
      case T_TRUE: lex.next(); return true;
      case T_FALSE: lex.next(); return false;
      case T_NULL: lex.next(); return null;
    }
    throw new Error("parse error: unexpected token " + lex.kind);
  }
  parseObject() {
    const obj = {};
    this.expect(T_LBRACE);
    if (this.lex.kind === T_RBRACE) {
      this.lex.next();
      return obj;
    }
    for (;;) {
      if (this.lex.kind !== T_STRING) throw new Error("parse error: key");
      const key = this.lex.value;
      this.lex.next();
      this.expect(T_COLON);
      obj[key] = this.parseValue();
      if (this.lex.kind === T_COMMA) {
        this.lex.next();
        continue;
      }
      this.expect(T_RBRACE);
      return obj;
    }
  }
  parseArray() {
    const arr = [];
    this.expect(T_LBRACK);
    if (this.lex.kind === T_RBRACK) {
      this.lex.next();
      return arr;
    }
    for (;;) {
      arr.push(this.parseValue());
      if (this.lex.kind === T_COMMA) {
        this.lex.next();
        continue;
      }
      this.expect(T_RBRACK);
      return arr;
    }
  }
  parseDocument() {
    const v = this.parseValue();
    if (this.lex.kind !== T_EOF) throw new Error("trailing garbage");
    return v;
  }
}

// --- serializer ---
function quote(s) {
  let out = "\"";
  let start = 0;
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    if (c === 34 || c === 92 || c === 10) {
      out += s.slice(start, i);
      out += c === 34 ? "\\\"" : (c === 92 ? "\\\\" : "\\n");
      start = i + 1;
    }
  }
  return out + s.slice(start) + "\"";
}

function serialize(v) {
  if (v === null) return "null";
  const t = typeof v;
  if (t === "number") return String(v);
  if (t === "boolean") return v ? "true" : "false";
  if (t === "string") return quote(v);
  if (Array.isArray(v)) return "[" + v.map(serialize).join(",") + "]";
  const parts = [];
  for (const k of Object.keys(v)) parts.push(quote(k) + ":" + serialize(v[k]));
  return "{" + parts.join(",") + "}";
}

// --- walker: structural checksum ---
function walk(v, stats) {
  if (v === null) {
    stats.nulls++;
    return;
  }
  const t = typeof v;
  if (t === "number") {
    stats.numbers++;
    stats.numSum += v;
  } else if (t === "string") {
    stats.strings++;
    stats.chars += v.length;
  } else if (t === "boolean") {
    if (v) stats.trues++;
  } else if (Array.isArray(v)) {
    stats.arrays++;
    for (const e of v) walk(e, stats);
  } else {
    stats.objects++;
    for (const k in v) {
      stats.keys += k.length;
      walk(v[k], stats);
    }
  }
}

function newStats() {
  return { nulls: 0, numbers: 0, numSum: 0, strings: 0, chars: 0, trues: 0,
           arrays: 0, objects: 0, keys: 0 };
}

const text = genDocument(DOC_ITEMS);
let check = 0;
let lastStats = null;
let lastLen = 0;
for (let r = 0; r < ROUNDS; r++) {
  const doc = new Parser(text).parseDocument();
  const out = serialize(doc);
  const doc2 = new Parser(out).parseDocument();
  const out2 = serialize(doc2);
  if (out2 !== out) throw new Error("round trip mismatch");
  const stats = newStats();
  walk(doc2, stats);
  check = (check + stats.objects * 7 + stats.chars + out.length) | 0;
  lastStats = stats;
  lastLen = out.length;
}
console.log("json", ROUNDS, text.length, lastLen, check);
console.log("stats", lastStats.objects, lastStats.arrays, lastStats.strings,
            lastStats.numbers, lastStats.nulls, lastStats.trues, lastStats.keys,
            lastStats.numSum.toFixed(3));
