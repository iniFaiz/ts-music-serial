// Named, auto-updating "insight" collections shown on the Home page and opened
// full-screen by CollectionDetail. Each one is derived live from the library +
// play stats (via store getters) so it always reflects current listening.
//
// `rules`/`sortBy`/`sortOrder` mirror the smart-playlist shape so a collection
// can be saved as an editable Smart Playlist with one click ("open the door"
// from a stat view into a real smart playlist).

export const COLLECTIONS = {
  'recently-played': {
    key: 'recently-played',
    title: 'Recently Played',
    subtitle: 'Pick up where you left off',
    color: '#1fb6ff,#0b3d91',
    icon: 'clock',
    songs: (store) => store.recentlyPlayedSongs,
    rules: { match: 'all', conditions: [{ field: 'lastPlayed', op: 'played', value: '' }] },
    sortBy: 'lastPlayed',
    sortOrder: 'desc',
  },
  'on-repeat': {
    key: 'on-repeat',
    title: 'On Repeat',
    subtitle: "Songs you can't stop playing",
    color: '#ff6a3d,#b81d54',
    icon: 'repeat',
    songs: (store) => store.onRepeatSongs,
    rules: {
      match: 'all',
      conditions: [
        { field: 'playCount', op: 'gte', value: 2 },
        { field: 'lastPlayed', op: 'inLast', value: 45 },
      ],
    },
    sortBy: 'playCount',
    sortOrder: 'desc',
  },
  'most-played': {
    key: 'most-played',
    title: 'Most Played',
    subtitle: 'Your all-time top tracks',
    color: '#8e44ff,#2d1b69',
    icon: 'fire',
    songs: (store) => store.mostPlayedSongs,
    rules: { match: 'all', conditions: [{ field: 'playCount', op: 'gte', value: 1 }] },
    sortBy: 'playCount',
    sortOrder: 'desc',
  },
  'recently-added': {
    key: 'recently-added',
    title: 'Recently Added',
    subtitle: 'Fresh in your library',
    color: '#19c37d,#0b6e4f',
    icon: 'sparkles',
    songs: (store) => store.recentlyAddedSongs,
    rules: { match: 'all', conditions: [] },
    sortBy: 'dateAdded',
    sortOrder: 'desc',
  },

  rediscover: {
    key: 'rediscover',
    title: 'Rediscover',
    subtitle: "Loved songs you've missed lately",
    color: '#36d1dc,#1a4a8a',
    icon: 'heart',
    songs: (store) => store.rediscoverSongs,
    rules: {
      match: 'all',
      conditions: [
        { field: 'favorite', op: 'isTrue', value: '' },
        { field: 'lastPlayed', op: 'notInLast', value: 60 },
      ],
    },
    sortBy: 'random',
    sortOrder: 'asc',
  },
};

export function getCollection(key) {
  return COLLECTIONS[key] || null;
}

// Order the big "Top Picks" cards appear in on Home.
export const TOP_PICKS_ORDER = [
  'on-repeat',
  'most-played',
  'recently-added',
  'rediscover',
  'recently-played',
];
