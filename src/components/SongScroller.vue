<script setup>
import { useRouter } from 'vue-router';
import { store } from '../store';
import CoverImage from './CoverImage.vue';
import Shelf from './Shelf.vue';
import { navigateWithTransition } from '../viewTransition';

const props = defineProps({
  title: { type: String, required: true },
  subtitle: { type: String, default: '' },
  songs: { type: Array, default: () => [] },
  // router-link target for the "see all" chevron + heading.
  to: { type: [String, Object], default: null },
  // Show a large rank number over the cover (Most Played style).
  showRank: { type: Boolean, default: false },
  // How many cards to render in the row.
  limit: { type: Number, default: 24 },
});

const router = useRouter();

const shown = () => props.songs.slice(0, props.limit);

const play = (song) => {
  store.playSong(song, props.songs);
};

const isCurrent = (song) => store.currentSong && store.currentSong.path === song.path;

// Navigate to the artist, morphing this card's cover into the artist photo.
const goToArtist = (artist, event) => {
  if (!artist || artist === 'Unknown Artist') return;
  const card = event.currentTarget.closest('.song-card');
  const coverEl = card ? card.querySelector('.cover-image') : null;
  const navigate = () => router.push({ name: 'ArtistDetail', params: { name: artist } });
  if (coverEl) navigateWithTransition(navigate, coverEl, 'shared-cover', 'to-artist-transition');
  else navigate();
};
</script>

<template>
  <Shelf v-if="songs.length > 0" :title="title" :subtitle="subtitle" :to="to">
    <div
      v-for="(song, idx) in shown()"
      :key="song.path"
      :data-artist-key="song.artist"
      class="song-card shrink-0 w-40 group cursor-pointer"
      @click="play(song)"
    >
      <div class="relative w-40 h-40 mb-2.5 rounded-xl overflow-hidden shadow-lg group-hover:scale-[1.03] transition-transform duration-200 ease-out">
        <CoverImage :path="song.path" className="w-full h-full bg-[#282828]" />

        <!-- Rank number -->
        <div
          v-if="showRank"
          class="absolute top-1.5 left-2 text-3xl font-black text-white drop-shadow-[0_2px_4px_rgba(0,0,0,0.7)] leading-none"
        >
          {{ idx + 1 }}
        </div>

        <!-- Play overlay -->
        <div class="absolute inset-0 bg-black/25 opacity-0 group-hover:opacity-100 transition-opacity flex items-end justify-end p-2.5">
          <div
            class="bg-[var(--accent-color)] text-white rounded-full p-2.5 shadow-xl translate-y-2 opacity-0 group-hover:translate-y-0 group-hover:opacity-100 transition-all duration-300 hover:scale-110 hover:bg-red-500"
          >
            <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="currentColor" stroke="none">
              <polygon points="5 3 19 12 5 21 5 3" />
            </svg>
          </div>
        </div>
      </div>
      <div
        class="text-[13px] font-semibold truncate leading-tight"
        :class="isCurrent(song) ? 'text-[var(--accent-color)]' : 'text-white'"
      >
        {{ song.title }}
      </div>
      <div class="text-[12px] text-[var(--text-secondary)] truncate">
        <span
          @click.stop="goToArtist(song.artist, $event)"
          class="hover:text-[var(--accent-color)] hover:underline cursor-pointer transition-colors"
        >
          {{ song.artist }}
        </span>
      </div>
    </div>
  </Shelf>
</template>
