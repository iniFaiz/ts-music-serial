// Smart-playlist rule engine.
//
// A smart playlist is a small, serializable description of *rules* rather than a
// fixed list of tracks. It is re-evaluated against the live library every time
// it is shown, so it "auto-updates" as the library, ratings and play stats
// change — just like Apple Music / iTunes smart playlists.
//
// Shape:
//   {
//     id, name, description, color,
//     rules: { match: 'all' | 'any', conditions: [{ field, op, value }] },
//     limit: number | null,          // 0 / null = unlimited
//     sortBy: <field key> | 'random' | 'none',
//     sortOrder: 'asc' | 'desc',
//     liveUpdate: boolean,           // auto-refresh (purely informational here)
//   }
//
// Evaluation is pure: it takes the songs array plus a small `ctx` adapter that
// exposes the bits of state that don't live on the song object itself (ratings,
// play stats, favorites, current time). The store builds that ctx.

// ---- Field catalogue --------------------------------------------------------

export const FIELDS = [
  { key: 'title', label: 'Title', type: 'text' },
  { key: 'artist', label: 'Artist', type: 'text' },
  { key: 'album', label: 'Album', type: 'text' },
  { key: 'genre', label: 'Genre', type: 'text' },
  { key: 'year', label: 'Year', type: 'number' },
  { key: 'playCount', label: 'Play Count', type: 'number' },
  { key: 'lastPlayed', label: 'Last Played', type: 'date' },
  { key: 'dateAdded', label: 'Date Added', type: 'date' },
  { key: 'duration', label: 'Duration (sec)', type: 'duration' },
  { key: 'favorite', label: 'Loved', type: 'bool' },
];

export const FIELD_MAP = Object.fromEntries(FIELDS.map((f) => [f.key, f]));

// Operators offered per field type. `value` indicates whether the operator needs
// a value input (false for things like "is loved" / "never played").
export const OPERATORS = {
  text: [
    { op: 'contains', label: 'contains', value: true },
    { op: 'notContains', label: 'does not contain', value: true },
    { op: 'is', label: 'is', value: true },
    { op: 'isNot', label: 'is not', value: true },
    { op: 'startsWith', label: 'starts with', value: true },
    { op: 'endsWith', label: 'ends with', value: true },
  ],
  number: [
    { op: 'is', label: 'is', value: true },
    { op: 'isNot', label: 'is not', value: true },
    { op: 'gt', label: 'is greater than', value: true },
    { op: 'lt', label: 'is less than', value: true },
    { op: 'gte', label: 'is at least', value: true },
    { op: 'lte', label: 'is at most', value: true },
  ],
  duration: [
    { op: 'lt', label: 'is shorter than', value: true },
    { op: 'gt', label: 'is longer than', value: true },
  ],
  date: [
    { op: 'inLast', label: 'in the last (days)', value: true },
    { op: 'notInLast', label: 'not in the last (days)', value: true },
    { op: 'played', label: 'has been played', value: false },
    { op: 'never', label: 'never played', value: false },
  ],
  bool: [
    { op: 'isTrue', label: 'is true', value: false },
    { op: 'isFalse', label: 'is false', value: false },
  ],
};

export function operatorsFor(fieldKey) {
  const f = FIELD_MAP[fieldKey];
  return f ? OPERATORS[f.type] || [] : [];
}

export function operatorNeedsValue(fieldKey, op) {
  const list = operatorsFor(fieldKey);
  const found = list.find((o) => o.op === op);
  return found ? found.value : true;
}

// Sort options shown in the editor (a subset of fields + special modes).
export const SORT_OPTIONS = [
  { key: 'none', label: 'Custom (no sort)' },
  { key: 'random', label: 'Random' },
  { key: 'title', label: 'Title' },
  { key: 'artist', label: 'Artist' },
  { key: 'album', label: 'Album' },
  { key: 'year', label: 'Year' },
  { key: 'playCount', label: 'Play Count' },
  { key: 'lastPlayed', label: 'Last Played' },
  { key: 'dateAdded', label: 'Date Added' },
  { key: 'duration', label: 'Duration' },
];

// ---- Value extraction -------------------------------------------------------

// Resolve a field's comparable value for a song. Dates are returned as ms epoch
// (0 = "never"); booleans as real booleans; everything else as string/number.
function fieldValue(fieldKey, song, ctx) {
  switch (fieldKey) {
    case 'title':
      return song.title || '';
    case 'artist':
      return song.artist || '';
    case 'album':
      return song.album || '';
    case 'genre':
      return song.genre || '';
    case 'year':
      return song.year || 0;
    case 'duration':
      return song.duration_secs || 0;
    case 'playCount':
      return ctx.stat(song.path).playCount || 0;
    case 'lastPlayed':
      return ctx.stat(song.path).lastPlayed || 0;
    case 'dateAdded':
      return (song.date_added || 0) * 1000;
    case 'favorite':
      return ctx.isFavorite(song.path);
    default:
      return '';
  }
}

function matchCondition(cond, song, ctx) {
  const field = FIELD_MAP[cond.field];
  if (!field || !cond.op) return true;
  const v = fieldValue(cond.field, song, ctx);

  if (field.type === 'text') {
    const a = String(v).toLowerCase();
    const b = String(cond.value ?? '').toLowerCase();
    switch (cond.op) {
      case 'contains':
        return a.includes(b);
      case 'notContains':
        return !a.includes(b);
      case 'is':
        return a === b;
      case 'isNot':
        return a !== b;
      case 'startsWith':
        return a.startsWith(b);
      case 'endsWith':
        return a.endsWith(b);
      default:
        return true;
    }
  }

  if (field.type === 'number' || field.type === 'duration') {
    const a = Number(v) || 0;
    const b = Number(cond.value) || 0;
    switch (cond.op) {
      case 'is':
        return a === b;
      case 'isNot':
        return a !== b;
      case 'gt':
        return a > b;
      case 'lt':
        return a < b;
      case 'gte':
        return a >= b;
      case 'lte':
        return a <= b;
      default:
        return true;
    }
  }

  if (field.type === 'date') {
    const a = Number(v) || 0; // 0 = never
    const days = Number(cond.value) || 0;
    const cutoff = ctx.now - days * 86400000;
    switch (cond.op) {
      case 'inLast':
        return a > 0 && a >= cutoff;
      case 'notInLast':
        return a === 0 || a < cutoff;
      case 'played':
        return a > 0;
      case 'never':
        return a === 0;
      default:
        return true;
    }
  }

  if (field.type === 'bool') {
    const a = !!v;
    return cond.op === 'isFalse' ? a === false : a === true;
  }

  return true;
}

// ---- Public evaluation ------------------------------------------------------

export function evaluateRules(rules, songs, ctx) {
  const conditions = (rules?.conditions || []).filter((c) => c && c.field && c.op);
  if (conditions.length === 0) return [...songs];
  const matchAll = (rules?.match || 'all') === 'all';
  return songs.filter((song) =>
    matchAll
      ? conditions.every((c) => matchCondition(c, song, ctx))
      : conditions.some((c) => matchCondition(c, song, ctx))
  );
}

export function sortSongs(songs, sortBy, sortOrder, ctx) {
  if (!sortBy || sortBy === 'none') return songs;
  if (sortBy === 'random') {
    const arr = [...songs];
    for (let i = arr.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [arr[i], arr[j]] = [arr[j], arr[i]];
    }
    return arr;
  }
  const dir = sortOrder === 'desc' ? -1 : 1;
  return [...songs].sort((a, b) => {
    let va = fieldValue(sortBy, a, ctx);
    let vb = fieldValue(sortBy, b, ctx);
    if (typeof va === 'string') va = va.toLowerCase();
    if (typeof vb === 'string') vb = vb.toLowerCase();
    if (va < vb) return -1 * dir;
    if (va > vb) return 1 * dir;
    return 0;
  });
}

export function evaluateSmartPlaylist(sp, songs, ctx) {
  let result = evaluateRules(sp.rules, songs, ctx);
  result = sortSongs(result, sp.sortBy, sp.sortOrder, ctx);
  const limit = Number(sp.limit) || 0;
  if (limit > 0) result = result.slice(0, limit);
  return result;
}

// ---- Human-readable summary -------------------------------------------------

function describeCondition(cond) {
  const field = FIELD_MAP[cond.field];
  if (!field) return '';
  const opDef = operatorsFor(cond.field).find((o) => o.op === cond.op);
  const opLabel = opDef ? opDef.label : cond.op;
  if (opDef && !opDef.value) return `${field.label} ${opLabel}`;
  let val = cond.value;
  return `${field.label} ${opLabel} ${val ?? ''}`.trim();
}

export function describeRules(sp) {
  const conds = (sp?.rules?.conditions || []).filter((c) => c && c.field && c.op);
  if (conds.length === 0) return 'All songs';
  const joiner = (sp.rules?.match || 'all') === 'all' ? ' and ' : ' or ';
  return conds.map(describeCondition).join(joiner);
}

// ---- Factory + templates ----------------------------------------------------

let counter = 0;
export function genSmartId() {
  counter += 1;
  return 'sp_' + Date.now().toString(36) + counter.toString(36) + Math.random().toString(36).slice(2, 6);
}

export function newSmartPlaylist(overrides = {}) {
  return {
    id: genSmartId(),
    name: 'New Smart Playlist',
    description: '',
    color: pickColor(),
    cover: null,
    paths: [], // unused by smart playlists, kept so library-pruning stays uniform
    rules: { match: 'all', conditions: [{ field: 'artist', op: 'contains', value: '' }] },
    limit: 0,
    sortBy: 'none',
    sortOrder: 'asc',
    liveUpdate: true,
    ...overrides,
  };
}

// A warm palette of gradient pairs for the auto-generated cover art.
export const SMART_COLORS = [
  ['#fa2d48', '#7a1020'],
  ['#ff6a3d', '#b81d54'],
  ['#8e44ff', '#2d1b69'],
  ['#1fb6ff', '#0b3d91'],
  ['#19c37d', '#0b6e4f'],
  ['#ffb13d', '#b8531d'],
  ['#ff4d9d', '#7a1057'],
  ['#36d1dc', '#1a4a8a'],
];

function pickColor() {
  return SMART_COLORS[Math.floor(Math.random() * SMART_COLORS.length)].join(',');
}

// Starter templates the user can spin up with one click from the Home page.
export const SMART_TEMPLATES = [
  {
    name: 'Heavy Rotation',
    description: 'Tracks you keep coming back to.',
    color: '#ff6a3d,#b81d54',
    rules: { match: 'all', conditions: [{ field: 'playCount', op: 'gte', value: 5 }] },
    sortBy: 'playCount',
    sortOrder: 'desc',
    limit: 50,
  },
  {
    name: 'Fresh Finds',
    description: 'Added to your library in the last 30 days.',
    color: '#19c37d,#0b6e4f',
    rules: { match: 'all', conditions: [{ field: 'dateAdded', op: 'inLast', value: 30 }] },
    sortBy: 'dateAdded',
    sortOrder: 'desc',
    limit: 0,
  },
  {
    name: 'Rediscover',
    description: "Loved songs you haven't played in a while.",
    color: '#8e44ff,#2d1b69',
    rules: {
      match: 'all',
      conditions: [
        { field: 'favorite', op: 'isTrue', value: '' },
        { field: 'lastPlayed', op: 'notInLast', value: 60 },
      ],
    },
    sortBy: 'random',
    sortOrder: 'asc',
    limit: 50,
  },
];
