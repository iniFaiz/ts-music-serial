<script setup>
import { computed } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { useRouter } from 'vue-router';
import { store } from '../store';
import { useQuery } from '../useLibraryData';
import { COLLECTIONS, TOP_PICKS_ORDER, getCollection } from '../collections';
import { SMART_TEMPLATES } from '../smartPlaylists';
import SmartCover from '../components/SmartCover.vue';
import SongScroller from '../components/SongScroller.vue';
import CoverImage from '../components/CoverImage.vue';
import PlaylistCover from '../components/PlaylistCover.vue';
import Shelf from '../components/Shelf.vue';
import { navigateWithTransition, setMorphCollectionKey } from '../viewTransition';

defineOptions({ name: 'HomeView' });

const router = useRouter();

// Navigate to a detail page, morphing this card's cover into the destination
// art (shared-element transition, like the Albums/Artists tabs).
const openWithMorph = (to, event, klass = 'to-album-transition') => {
  // Remember morph intent for collection pages (so their cover morphs from this
  // card, unlike the header "see all" links which cross-fade).
  const path = typeof to === 'string' ? to : (to && to.path) || '';
  const m = typeof path === 'string' ? path.match(/^\/collection\/(.+)$/) : null;
  setMorphCollectionKey(m ? decodeURIComponent(m[1]) : null);

  const art = event.currentTarget.querySelector('.cover-image');
  const navigate = () => router.push(to);
  if (art) navigateWithTransition(navigate, art, 'shared-cover', klass);
  else navigateWithTransition(navigate, null);
};

// ---- Greeting ----
const greeting = computed(() => {
  const h = new Date().getHours();
  if (h < 5) return 'Good night';
  if (h < 12) return 'Good morning';
  if (h < 18) return 'Good afternoon';
  return 'Good evening';
});

const hasSongs = computed(() => store.scanCount > 0);

// ---- Insight song collections (fetched from the DB; refetch on stats change) --
const { data: recentlyPlayed } = useQuery(() => store.recentlyPlayed(60), {
  watchStats: true,
  initial: [],
});
const { data: onRepeat } = useQuery(() => store.onRepeat(60), { watchStats: true, initial: [] });
const { data: mostPlayed } = useQuery(() => store.mostPlayed(60), { watchStats: true, initial: [] });
const { data: recentlyAdded } = useQuery(() => store.recentlyAdded(60), { initial: [] });

// ---- Big "Top Picks" gradient cards ----
// Only need to know which collections currently have any tracks (the card shows
// no count), so this refetches with the library/stats.
const { data: topPicks } = useQuery(
  async () => {
    // One cheap COUNT query decides which collections are non-empty, instead of
    // fetching every collection's tracks just to check `.length`.
    const counts = await invoke('db_insight_counts');
    const byKey = {
      'recently-played': counts.recently_played,
      'on-repeat': counts.on_repeat,
      'most-played': counts.most_played,
      'recently-added': counts.recently_added,
      rediscover: counts.rediscover,
    };
    return TOP_PICKS_ORDER.map((k) => COLLECTIONS[k])
      .filter(Boolean)
      .map((c) => ({ ...c, count: byKey[c.key] || 0 }))
      .filter((c) => c.count > 0);
  },
  { watchStats: true, initial: [] }
);

const playCollection = async (key) => {
  const c = getCollection(key);
  if (!c) return;
  const l = await c.fetch(store);
  if (l.length) {
    store.recordRecent('collection', key);
    store.playSong(l[0], l);
  }
};

// ---- Smart playlists ----
const smartPlaylists = computed(() => store.smartPlaylists);
const smartCount = (sp) => sp.track_count || 0;

const createFromTemplate = async (t) => {
  const sp = await store.createSmartPlaylist({
    name: t.name,
    description: t.description,
    color: t.color,
    rules: JSON.parse(JSON.stringify(t.rules)),
    sortBy: t.sortBy,
    sortOrder: t.sortOrder,
    limit: t.limit,
  });
  if (sp) router.push('/smart/' + sp.id);
};

// ---- Stations ----
const { data: artistStations } = useQuery(() => store.topArtists(14), {
  watchStats: true,
  initial: [],
});
const { data: genreStations } = useQuery(() => store.topGenres(14), {
  watchStats: true,
  initial: [],
});
const hasStations = computed(() => artistStations.value.length > 0 || genreStations.value.length > 0);

// ---- Cover maps for resolving recent album/station cards ----
const { data: albumRows } = useQuery(() => invoke('db_albums', { search: null }), { initial: [] });
const albumCoverMap = computed(() => {
  const m = new Map();
  for (const a of albumRows.value) m.set(a.album, a.cover_path);
  return m;
});
const artistCoverMap = computed(() => {
  const m = new Map();
  for (const a of artistStations.value) m.set(a.name, a.coverPath);
  return m;
});

const resolveRecent = (r) => {
  if (r.type === 'playlist') {
    const pl = store.getPlaylist(r.key);
    if (!pl || pl.is_smart) return null;
    return { kind: 'playlist', id: pl.id, title: pl.name, sub: 'Playlist', cover: pl.cover, name: pl.name };
  }
  if (r.type === 'smart') {
    const sp = store.getSmartPlaylist(r.key);
    if (!sp) return null;
    return { kind: 'smart', id: sp.id, title: sp.name, sub: 'Smart Playlist', color: sp.color, cover: sp.cover };
  }
  if (r.type === 'collection') {
    const c = getCollection(r.key);
    if (!c) return null;
    return { kind: 'collection', key: r.key, title: c.title, sub: 'Mix', color: c.color, icon: c.icon };
  }
  if (r.type === 'album') {
    const cover = albumCoverMap.value.get(r.key);
    return { kind: 'album', name: r.key, title: r.key, sub: 'Album', coverPath: cover || null };
  }
  if (r.type === 'station') {
    const idx = r.key.indexOf(':');
    const stationType = r.key.slice(0, idx);
    const name = r.key.slice(idx + 1);
    return {
      kind: 'station',
      stationType,
      name,
      title: name,
      sub: stationType === 'genre' ? 'Genre Station' : 'Station',
      coverPath: stationType === 'artist' ? artistCoverMap.value.get(name) || null : null,
    };
  }
  return null;
};

const recentItems = computed(() => {
  const items = [];
  for (const r of store.recents) {
    const resolved = resolveRecent(r);
    if (resolved) items.push({ ...resolved, ts: r.ts });
  }
  // Recently-played songs come back newest-first; anchor synthetic timestamps so
  // they interleave with the container recents in that order.
  const now = Date.now();
  recentlyPlayed.value.slice(0, 14).forEach((s, i) => {
    items.push({ kind: 'song', song: s, title: s.title, sub: s.artist, ts: now - i });
  });
  items.sort((a, b) => b.ts - a.ts);
  return items.slice(0, 20);
});

const onRecentClick = (item, event) => {
  switch (item.kind) {
    case 'song':
      store.playSong(item.song, recentlyPlayed.value);
      break;
    case 'album':
      openWithMorph({ name: 'AlbumDetail', params: { name: item.name } }, event);
      break;
    case 'playlist':
      openWithMorph('/playlists/' + item.id, event);
      break;
    case 'smart':
      openWithMorph('/smart/' + item.id, event);
      break;
    case 'collection':
      openWithMorph('/collection/' + item.key, event);
      break;
    case 'station':
      store.playStation(item.stationType, item.name);
      break;
  }
};

const onRecentPlay = (item) => {
  switch (item.kind) {
    case 'song':
      store.playSong(item.song, recentlyPlayed.value);
      break;
    case 'album': {
      invoke('db_album_tracks', { album: item.name }).then((songs) => {
        if (songs.length) {
          store.recordRecent('album', item.name);
          store.playSong(songs[0], songs);
        }
      });
      break;
    }
    case 'playlist':
      store.playPlaylist(item.id);
      break;
    case 'smart':
      store.playSmartPlaylist(item.id);
      break;
    case 'collection':
      playCollection(item.key);
      break;
    case 'station':
      store.playStation(item.stationType, item.name);
      break;
  }
};

// Navigate to an artist from a recent song card, morphing the cover.
const goToArtist = (artist, event) => {
  if (!artist || artist === 'Unknown Artist') return;
  const card = event.currentTarget.closest('.rec-card');
  const art = card ? card.querySelector('.cover-image') : null;
  const navigate = () => router.push({ name: 'ArtistDetail', params: { name: artist } });
  if (art) navigateWithTransition(navigate, art, 'shared-cover', 'to-artist-transition');
  else navigate();
};
</script>

<template>
  <div class="h-full overflow-auto pb-16">
    <!-- Header -->
    <div class="px-8 pt-9 pb-6">
      <p class="text-sm font-medium text-[var(--accent-color)] mb-1">{{ greeting }}</p>
      <h1 class="text-4xl font-bold tracking-tight text-white">Home</h1>
    </div>

    <!-- Empty library state -->
    <div v-if="!hasSongs" class="px-8 py-20 text-center">
      <div class="mx-auto w-20 h-20 rounded-2xl bg-gradient-to-br from-[var(--accent-color)] to-[#7a1020] flex items-center justify-center mb-5 shadow-xl">
        <svg xmlns="http://www.w3.org/2000/svg" width="38" height="38" viewBox="0 0 24 24" fill="#fff" stroke="none">
          <path d="M9 18V5l12-2v13" /><circle cx="6" cy="18" r="3" /><circle cx="18" cy="16" r="3" />
        </svg>
      </div>
      <h2 class="text-xl font-bold text-white mb-1">Your library is empty</h2>
      <p class="text-sm text-gray-500 mb-6 max-w-sm mx-auto">
        Add a music folder to start building smart playlists and listening insights.
      </p>
      <button
        @click="store.selectAndScan()"
        class="bg-[var(--accent-color)] hover:bg-red-500 text-white font-semibold px-6 py-2.5 rounded-lg shadow-lg transition"
      >
        Add Music Folder
      </button>
    </div>

    <template v-else>
      <!-- Recently Played (mixed songs + playlists/stations/albums) -->
      <Shelf v-if="recentItems.length" title="Recently Played" to="/collection/recently-played">
        <div
          v-for="item in recentItems"
          :key="item.kind + '-' + (item.id || item.key || item.name || (item.song && item.song.path))"
          class="rec-card shrink-0 w-40 group cursor-pointer"
          :data-cover-key="item.kind === 'album' ? item.name : item.id || item.key || undefined"
          :data-artist-key="item.kind === 'song' ? item.sub : undefined"
          @click="onRecentClick(item, $event)"
        >
          <div
            class="relative w-40 h-40 mb-2.5 shadow-lg group-hover:scale-[1.03] transition-transform duration-200 ease-out"
            :class="item.kind === 'station' ? 'rounded-full' : 'rounded-xl'"
          >
            <!-- Art per kind -->
            <CoverImage
              v-if="item.kind === 'song' || item.kind === 'album' || (item.kind === 'station' && item.coverPath)"
              :path="item.kind === 'station' ? item.coverPath : (item.song ? item.song.path : item.coverPath)"
              :class="item.kind === 'station' ? 'w-full h-full !rounded-full' : 'w-full h-full rounded-xl'"
              className="bg-[#282828]"
            />
            <PlaylistCover
              v-else-if="item.kind === 'playlist' || item.kind === 'smart'"
              :name="item.title"
              :cover="item.cover"
              :size="160"
              className="w-full h-full rounded-xl bg-[#282828] cover-image"
            />
            <SmartCover
              v-else
              :title="item.title"
              :color="item.color || ''"
              :cover="item.cover"
              :icon="item.icon || (item.kind === 'station' ? 'radio' : 'bolt')"
              :show-title="false"
              :className="(item.kind === 'station' ? 'w-full h-full rounded-full' : 'w-full h-full rounded-xl') + ' cover-image'"
            />

            <!-- Play overlay -->
            <div
              class="absolute inset-0 bg-black/25 opacity-0 group-hover:opacity-100 transition-opacity flex items-end justify-end p-2.5"
              :class="item.kind === 'station' ? 'rounded-full' : 'rounded-xl'"
            >
              <div
                @click.stop="onRecentPlay(item)"
                class="bg-[var(--accent-color)] text-white rounded-full p-2.5 shadow-xl translate-y-2 opacity-0 group-hover:translate-y-0 group-hover:opacity-100 transition-all duration-300 hover:scale-110 hover:bg-red-500"
              >
                <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="currentColor" stroke="none">
                  <polygon points="5 3 19 12 5 21 5 3" />
                </svg>
              </div>
            </div>
          </div>
          <div
            class="text-[13px] font-semibold text-white truncate leading-tight"
            :class="item.kind === 'station' ? 'text-center' : ''"
          >
            {{ item.title }}
          </div>
          <div
            class="text-[12px] text-[var(--text-secondary)] truncate"
            :class="item.kind === 'station' ? 'text-center' : ''"
          >
            <span
              v-if="item.kind === 'song'"
              @click.stop="goToArtist(item.sub, $event)"
              class="hover:text-[var(--accent-color)] hover:underline cursor-pointer transition-colors"
            >
              {{ item.sub }}
            </span>
            <span v-else>{{ item.sub }}</span>
          </div>
        </div>
      </Shelf>

      <!-- Top Picks (big gradient cards) -->
      <Shelf v-if="topPicks.length" title="Top Picks for You">
        <button
          v-for="c in topPicks"
          :key="c.key"
          :data-cover-key="c.key"
          @click="openWithMorph('/collection/' + c.key, $event)"
          class="shrink-0 w-64 text-left group"
        >
          <div class="w-64 aspect-[4/5] rounded-2xl overflow-hidden shadow-xl group-hover:scale-[1.02] transition-transform duration-200 ease-out relative">
            <SmartCover
              :title="c.title"
              top-label="Made for You"
              :subtitle="c.subtitle"
              :color="c.color"
              :icon="c.icon"
              className="w-full h-full cover-image"
            />
            <div
              @click.stop="playCollection(c.key)"
              class="absolute bottom-3 right-3 bg-white text-black rounded-full p-3 shadow-xl opacity-0 translate-y-2 group-hover:opacity-100 group-hover:translate-y-0 transition-all duration-300 hover:scale-110"
            >
              <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="currentColor" stroke="none">
                <polygon points="5 3 19 12 5 21 5 3" />
              </svg>
            </div>
          </div>
        </button>
      </Shelf>

      <!-- On Repeat -->
      <SongScroller title="On Repeat" :songs="onRepeat" to="/collection/on-repeat" />

      <!-- Most Played (ranked) -->
      <SongScroller title="Most Played" :songs="mostPlayed" to="/collection/most-played" show-rank />

      <!-- Smart Playlists -->
      <Shelf title="Your Smart Playlists">
        <button
          v-for="sp in smartPlaylists"
          :key="sp.id"
          :data-cover-key="sp.id"
          @click="openWithMorph('/smart/' + sp.id, $event)"
          class="shrink-0 w-48 text-left group"
        >
          <div class="w-48 h-48 rounded-2xl overflow-hidden shadow-xl group-hover:scale-[1.03] transition-transform duration-200 ease-out relative mb-2.5">
            <PlaylistCover :name="sp.name" :cover="sp.cover" :size="192" className="w-full h-full cover-image" />
            <div
              @click.stop="store.playSmartPlaylist(sp.id)"
              class="absolute bottom-2.5 right-2.5 bg-white text-black rounded-full p-2.5 shadow-xl opacity-0 translate-y-2 group-hover:opacity-100 group-hover:translate-y-0 transition-all duration-300 hover:scale-110"
            >
              <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="currentColor" stroke="none">
                <polygon points="5 3 19 12 5 21 5 3" />
              </svg>
            </div>
          </div>
          <div class="text-[13px] font-semibold text-white truncate flex items-center gap-1.5">
            <span class="truncate">{{ sp.name }}</span>
            <svg xmlns="http://www.w3.org/2000/svg" width="11" height="11" viewBox="0 0 24 24" fill="currentColor" stroke="none" class="text-[var(--accent-color)] shrink-0">
              <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" />
            </svg>
          </div>
          <div class="text-[12px] text-[var(--text-secondary)]">{{ smartCount(sp) }} songs</div>
        </button>

        <!-- Quick-create templates (only when the user has none yet) -->
        <template v-if="smartPlaylists.length === 0">
          <button
            v-for="t in SMART_TEMPLATES"
            :key="t.name"
            @click="createFromTemplate(t)"
            class="shrink-0 w-48 text-left group"
          >
            <div class="w-48 h-48 rounded-2xl overflow-hidden shadow-xl group-hover:scale-[1.03] transition-transform duration-200 ease-out relative mb-2.5">
              <PlaylistCover :name="t.name" :size="192" className="w-full h-full" />
              <div class="absolute inset-0 bg-black/30 opacity-0 group-hover:opacity-100 transition-opacity flex items-center justify-center">
                <div class="bg-white/90 text-black rounded-full p-2.5 shadow-xl scale-90 group-hover:scale-100 transition-transform">
                  <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" stroke-linejoin="round">
                    <line x1="12" y1="5" x2="12" y2="19" /><line x1="5" y1="12" x2="19" y2="12" />
                  </svg>
                </div>
              </div>
            </div>
            <div class="text-[13px] font-semibold text-white truncate">{{ t.name }}</div>
            <div class="text-[12px] text-[var(--text-secondary)]">Tap to create</div>
          </button>
        </template>

        <!-- New smart playlist card. self-start so its square top-aligns with the
             other cards' covers instead of centering against their taller
             (cover + label) height. -->
        <button @click="store.openSmartModal('create')" class="shrink-0 w-48 group self-start">
          <div class="w-48 h-48 rounded-2xl border-2 border-dashed border-white/15 group-hover:border-[var(--accent-color)] flex flex-col items-center justify-center gap-2 text-gray-500 group-hover:text-[var(--accent-color)] transition-colors mb-2.5">
            <svg xmlns="http://www.w3.org/2000/svg" width="30" height="30" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <line x1="12" y1="5" x2="12" y2="19" /><line x1="5" y1="12" x2="19" y2="12" />
            </svg>
            <span class="text-xs font-semibold">New Smart Playlist</span>
          </div>
        </button>
      </Shelf>

      <!-- Recently Added -->
      <SongScroller title="Recently Added" :songs="recentlyAdded" to="/collection/recently-added" />

      <!-- Stations for You -->
      <Shelf v-if="hasStations" title="Stations for You" subtitle="Endless mixes built from your library">
        <!-- Artist stations (circular) -->
        <button
          v-for="a in artistStations"
          :key="'a-' + a.name"
          @click="store.playStation('artist', a.name)"
          class="shrink-0 w-36 group text-center"
        >
          <div class="w-36 h-36 rounded-full overflow-hidden shadow-xl group-hover:scale-[1.04] transition-transform duration-200 ease-out relative mb-2.5 mx-auto">
            <CoverImage :path="a.coverPath" className="w-full h-full bg-[#282828]" />
            <div class="absolute inset-0 bg-black/30 opacity-0 group-hover:opacity-100 transition-opacity flex items-center justify-center">
              <div class="bg-[var(--accent-color)] text-white rounded-full p-3 shadow-xl scale-90 group-hover:scale-100 transition-transform hover:bg-red-500">
                <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="currentColor" stroke="none">
                  <polygon points="5 3 19 12 5 21 5 3" />
                </svg>
              </div>
            </div>
          </div>
          <div class="text-[13px] font-semibold text-white truncate">{{ a.name }}</div>
          <div class="text-[12px] text-[var(--text-secondary)]">Station</div>
        </button>

        <!-- Genre stations (gradient) -->
        <button
          v-for="g in genreStations"
          :key="'g-' + g.name"
          @click="store.playStation('genre', g.name)"
          class="shrink-0 w-36 group text-center"
        >
          <div class="w-36 h-36 rounded-2xl overflow-hidden shadow-xl group-hover:scale-[1.04] transition-transform duration-200 ease-out relative mb-2.5">
            <SmartCover :title="g.name" :color="''" icon="radio" :show-title="false" className="w-full h-full" />
            <div class="absolute inset-0 opacity-0 group-hover:opacity-100 transition-opacity flex items-center justify-center">
              <div class="bg-white text-black rounded-full p-3 shadow-xl scale-90 group-hover:scale-100 transition-transform">
                <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="currentColor" stroke="none">
                  <polygon points="5 3 19 12 5 21 5 3" />
                </svg>
              </div>
            </div>
          </div>
          <div class="text-[13px] font-semibold text-white truncate">{{ g.name }}</div>
          <div class="text-[12px] text-[var(--text-secondary)]">Genre Station</div>
        </button>
      </Shelf>
    </template>
  </div>
</template>
