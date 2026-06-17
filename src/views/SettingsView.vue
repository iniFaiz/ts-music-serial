<template>
  <div class="p-6 max-w-2xl mx-auto pb-16">
    <h1 class="text-2xl font-bold text-white mb-6">Settings</h1>

    <!-- Music Folders -->
    <Section
      title="Music Folders"
      description="Folders scanned for music. The library auto-updates when files change on disk."
    >
      <button
        @click="store.selectAndScan()"
        :disabled="store.loading"
        class="text-sm font-semibold text-[var(--accent-color)] hover:underline disabled:opacity-50"
      >
        + Add new folder
      </button>

      <div class="mt-4">
        <div class="text-xs uppercase tracking-wider text-gray-500 mb-2">Added folders</div>
        <div v-if="store.roots.length === 0" class="text-sm text-gray-500 py-2">
          No folders added yet.
        </div>
        <ul v-else class="space-y-1">
          <li
            v-for="root in store.roots"
            :key="root"
            class="flex items-center justify-between gap-3 py-2 px-2 rounded-md hover:bg-white/5 group"
          >
            <div class="flex items-center gap-3 min-w-0">
              <svg
                xmlns="http://www.w3.org/2000/svg"
                width="20"
                height="20"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="1.8"
                class="text-gray-400 shrink-0"
              >
                <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
              </svg>
              <span class="text-sm text-gray-200 truncate" :title="root">{{ root }}</span>
            </div>
            <button
              @click="confirmRemoveRoot(root)"
              class="text-sm font-medium text-gray-500 hover:text-[var(--accent-color)] shrink-0"
            >
              Remove
            </button>
          </li>
        </ul>
      </div>

      <div class="flex items-center gap-5 mt-4 pt-3 border-t border-white/5">
        <button
          @click="store.refreshLibrary()"
          :disabled="store.loading || store.roots.length === 0"
          class="text-sm font-medium text-gray-300 hover:text-white disabled:opacity-40"
        >
          Refresh
        </button>
        <button
          @click="store.reindexLibrary()"
          :disabled="store.loading || store.roots.length === 0"
          class="text-sm font-medium text-gray-300 hover:text-white disabled:opacity-40"
        >
          Reindex
        </button>
        <span class="text-xs text-gray-500 truncate">{{ store.statusMessage }}</span>
      </div>
    </Section>

    <!-- Audio Output -->
    <Section title="Audio Output" description="Choose which device audio is played through.">
      <SelectInt
        label="Output device"
        :modelValue="store.outputDevice || ''"
        :options="deviceOptions"
        @update:modelValue="onDeviceChange"
      />
      <button
        @click="loadDevices"
        class="text-xs font-medium text-gray-400 hover:text-white mt-1"
      >
        Refresh devices
      </button>
    </Section>

    <!-- Playback -->
    <Section title="Playback">
      <SelectInt
        label="Track transition"
        :modelValue="store.transitionMode"
        :options="transitionOptions"
        @update:modelValue="(v) => store.setTransitionMode(v)"
      />
      <SliderInt
        v-if="store.transitionMode === 'crossfade'"
        label="Crossfade duration"
        :modelValue="store.crossfadeSecs"
        :min="1"
        :max="12"
        :step="1"
        suffix="s"
        @update:modelValue="(v) => store.setCrossfadeSecs(v)"
      />
      <p class="text-xs text-gray-500 -mt-1 mb-2">
        Gapless pre-decodes the next track for seamless transitions. Crossfade overlaps the end of
        one track with the start of the next.
      </p>

      <div class="border-t border-white/5 pt-1">
        <ToggleInt
          :modelValue="store.normalizationEnabled"
          @update:modelValue="(v) => store.setNormalizationEnabled(v)"
          label="Volume normalization (Sound Check)"
        />
        <SliderInt
          v-if="store.normalizationEnabled"
          label="Pre-amp"
          :modelValue="store.normalizationPreampDb"
          :min="-12"
          :max="12"
          :step="1"
          suffix=" dB"
          @update:modelValue="(v) => store.setNormalizationPreamp(v)"
        />
        <p class="text-xs text-gray-500">
          Levels loudness across tracks using ReplayGain tags, falling back to an automatic
          loudness analysis (computed once per track in the background).
        </p>
      </div>

      <div class="border-t border-white/5 pt-1 mt-1">
        <ToggleInt
          :modelValue="store.visualizerEnabled"
          @update:modelValue="(v) => store.setVisualizerEnabled(v)"
          label="Audio visualizer"
        />
        <p class="text-xs text-gray-500">
          Real-time spectrum next to the player controls. Disable if you notice high CPU usage.
        </p>
      </div>
    </Section>

    <!-- Lyrics -->
    <Section
      title="Lyrics"
      description="Choose your preferred lyrics provider. You can search from LRCLIB, local tags/files, NetEase, Musixmatch, or disable lyrics search entirely."
    >
      <SelectInt
        label="Lyrics source"
        :modelValue="store.lyricsSource"
        :options="lyricsOptions"
        @update:modelValue="(v) => store.setLyricsSource(v)"
      />
      <div v-if="store.lyricsSource === 'musixmatch'" class="mt-3">
        <label class="text-sm text-gray-300 font-medium block mb-2">Musixmatch user token (optional)</label>
        <input
          :value="store.musixmatchToken"
          @change="(e) => store.setMusixmatchToken(e.target.value)"
          type="text"
          placeholder="Paste your Musixmatch community token"
          class="w-full bg-[#2a2a2a] text-sm text-white rounded-md px-3 py-2 focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)] placeholder-gray-600"
        />
      </div>
    </Section>

    <!-- Performance -->
    <Section title="Performance">
      <ToggleInt
        :modelValue="store.useParallelism"
        @update:modelValue="(v) => store.setParallelism(v)"
        label="Use parallel processing (faster scans)"
      />
      <p class="text-xs text-gray-500">
        Uses multiple CPU threads to scan and parse music files. Disable if you experience
        instability during scans.
      </p>
    </Section>

    <!-- Library -->
    <Section title="Library">
      <div class="flex items-center justify-between">
        <div>
          <h3 class="text-white font-medium text-sm">Reset library</h3>
          <p class="text-xs text-gray-500">Clear all songs, albums, playlists and likes.</p>
        </div>
        <button
          @click="confirmReset"
          class="px-4 py-2 bg-red-600 hover:bg-red-700 text-white rounded-md transition-colors text-sm font-medium shrink-0"
        >
          Reset Library
        </button>
      </div>
    </Section>
  </div>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue';
import { store } from '../store';
import { invoke } from '@tauri-apps/api/core';
import { confirm } from '@tauri-apps/plugin-dialog';
import Section from '../components/settings/Section.vue';
import ToggleInt from '../components/settings/ToggleInt.vue';
import SelectInt from '../components/settings/SelectInt.vue';
import SliderInt from '../components/settings/SliderInt.vue';

const devices = ref([]);

const deviceOptions = computed(() => [
  { value: '', label: 'System Default' },
  ...devices.value.map((d) => ({
    value: d.name,
    label: d.is_default ? `${d.name} (default)` : d.name,
  })),
]);

const transitionOptions = [
  { value: 'off', label: 'Off' },
  { value: 'gapless', label: 'Gapless' },
  { value: 'crossfade', label: 'Crossfade' },
];

const lyricsOptions = [
  { value: 'lrclib', label: 'LRCLIB (Default)' },
  { value: 'local', label: 'Local (Embedded tag / .lrc file)' },
  { value: 'netease', label: 'NetEase' },
  { value: 'musixmatch', label: 'Musixmatch' },
  { value: 'none', label: 'Off / Disabled' },
];

async function loadDevices() {
  try {
    devices.value = await invoke('list_output_devices');
  } catch (e) {
    console.error('Failed to list output devices', e);
    devices.value = [];
  }
}

function onDeviceChange(value) {
  store.setOutputDevice(value || null);
}

async function confirmRemoveRoot(root) {
  const yes = await confirm(`Remove "${root}" and its tracks from the library?`, {
    title: 'Remove Folder',
    kind: 'warning',
  });
  if (yes) store.removeRoot(root);
}

const confirmReset = async () => {
  const yes = await confirm(
    'Are you sure you want to delete all library data? This cannot be undone.',
    { title: 'Reset Library', kind: 'warning' }
  );
  if (yes) store.resetLibrary();
};

onMounted(loadDevices);
</script>
