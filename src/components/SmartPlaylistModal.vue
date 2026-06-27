<script setup>
import { ref, computed, watch } from 'vue';
import { useRouter } from 'vue-router';
import { store } from '../store';
import PlaylistCover from './PlaylistCover.vue';
import {
  FIELDS,
  FIELD_MAP,
  SORT_OPTIONS,
  operatorsFor,
  operatorNeedsValue,
  evaluateSmartPlaylist,
  newSmartPlaylist,
} from '../smartPlaylists';

const router = useRouter();

// Working copy — only committed to the store on Save.
const form = ref(newSmartPlaylist());

const initForm = () => {
  if (store.smartModal.mode === 'edit') {
    const existing = store.getSmartPlaylist(store.smartModal.smartId);
    form.value = existing
      ? JSON.parse(JSON.stringify(existing))
      : newSmartPlaylist();
  } else {
    form.value = newSmartPlaylist();
  }
  // Limit checkbox derives from the numeric limit.
  limitEnabled.value = Number(form.value.limit) > 0;
};

const limitEnabled = ref(false);

watch(
  () => store.smartModal.open,
  (open) => {
    if (open) initForm();
  }
);

// ---- Custom cover image ----
const fileInput = ref(null);
const pickImage = () => fileInput.value?.click();

const onFile = (e) => {
  const file = e.target.files && e.target.files[0];
  e.target.value = '';
  if (!file) return;
  const reader = new FileReader();
  reader.onload = () => downscale(String(reader.result));
  reader.readAsDataURL(file);
};

// Shrink the picked image so it stays small in storage.
const downscale = (dataUrl) => {
  const img = new Image();
  img.onload = () => {
    const max = 400;
    const scale = Math.min(1, max / Math.max(img.width, img.height));
    const w = Math.round(img.width * scale);
    const h = Math.round(img.height * scale);
    const canvas = document.createElement('canvas');
    canvas.width = w;
    canvas.height = h;
    canvas.getContext('2d').drawImage(img, 0, 0, w, h);
    form.value.cover = canvas.toDataURL('image/jpeg', 0.85);
  };
  img.src = dataUrl;
};

const clearCover = () => {
  form.value.cover = null;
};

// ---- Field / operator helpers (per condition row) ----
const fieldType = (cond) => FIELD_MAP[cond.field]?.type || 'text';
const opsFor = (cond) => operatorsFor(cond.field);
const needsValue = (cond) => operatorNeedsValue(cond.field, cond.op);

const onFieldChange = (cond) => {
  const ops = operatorsFor(cond.field);
  cond.op = ops.length ? ops[0].op : '';
  cond.value = '';
};

const addCondition = () => {
  form.value.rules.conditions.push({ field: 'artist', op: 'contains', value: '' });
};
const removeCondition = (i) => {
  form.value.rules.conditions.splice(i, 1);
};

// ---- Datalist suggestions from the library ----
const uniqueValues = (key) => {
  const set = new Set();
  for (const s of store.songs) {
    const v = s[key];
    if (v) set.add(v);
  }
  return [...set].sort((a, b) => String(a).localeCompare(String(b))).slice(0, 500);
};
const genreOptions = computed(() => uniqueValues('genre'));
const artistOptions = computed(() => uniqueValues('artist'));
const albumOptions = computed(() => uniqueValues('album'));

const datalistFor = (cond) => {
  if (cond.field === 'genre') return 'dl-genre';
  if (cond.field === 'artist') return 'dl-artist';
  if (cond.field === 'album') return 'dl-album';
  return undefined;
};

// ---- Live preview ----
const previewCount = computed(() => {
  const limit = limitEnabled.value ? Number(form.value.limit) || 0 : 0;
  const sp = { ...form.value, limit };
  return evaluateSmartPlaylist(sp, store.songs, store.insightCtx).length;
});

const showSortOrder = computed(
  () => form.value.sortBy && form.value.sortBy !== 'none' && form.value.sortBy !== 'random'
);

// Does any rule reference genre? (warn when the library lacks genre tags)
const usesGenre = computed(() =>
  (form.value.rules.conditions || []).some((c) => c.field === 'genre')
);
const libraryHasGenre = computed(() => store.songs.some((s) => s.genre));

const canSave = computed(() => (form.value.name || '').trim().length > 0);

const save = () => {
  if (!canSave.value) return;
  const payload = {
    name: form.value.name.trim(),
    description: (form.value.description || '').trim(),
    color: form.value.color,
    cover: form.value.cover || null,
    rules: form.value.rules,
    sortBy: form.value.sortBy,
    sortOrder: form.value.sortOrder,
    limit: limitEnabled.value ? Math.max(1, Number(form.value.limit) || 1) : 0,
    liveUpdate: form.value.liveUpdate,
  };
  if (store.smartModal.mode === 'edit') {
    store.updateSmartPlaylist(store.smartModal.smartId, payload);
    store.closeSmartModal();
  } else {
    const sp = store.createSmartPlaylist(payload);
    store.closeSmartModal();
    router.push('/smart/' + sp.id);
  }
};

const cancel = () => store.closeSmartModal();
</script>

<template>
  <Teleport to="body">
    <Transition name="modal">
      <div
        v-if="store.smartModal.open"
        class="fixed inset-0 z-[300] flex items-center justify-center bg-black/70 backdrop-blur-md"
        @click="cancel"
      >
        <div
          class="modal-panel relative w-[94%] max-w-2xl max-h-[88vh] flex flex-col bg-[#1c1c1e] border border-[#2c2c2e] rounded-2xl shadow-2xl overflow-hidden"
          @click.stop
        >
        <!-- Header -->
        <div class="flex items-center justify-between px-6 py-4 border-b border-[#2c2c2e] shrink-0">
          <div class="flex items-center gap-2.5">
            <div class="w-8 h-8 rounded-lg bg-[var(--accent-color)]/15 flex items-center justify-center text-[var(--accent-color)]">
              <svg xmlns="http://www.w3.org/2000/svg" width="17" height="17" viewBox="0 0 24 24" fill="currentColor" stroke="none">
                <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" />
              </svg>
            </div>
            <h2 class="text-lg font-bold text-white">
              {{ store.smartModal.mode === 'edit' ? 'Edit Smart Playlist' : 'New Smart Playlist' }}
            </h2>
          </div>
          <button @click="cancel" class="text-gray-400 hover:text-white transition-colors">
            <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
            </svg>
          </button>
        </div>

        <!-- Body (scrolls) -->
        <div class="px-6 py-5 overflow-y-auto scrollbar-thin space-y-5">
          <!-- Cover + name + description (matches the normal playlist editor) -->
          <div class="flex gap-5 items-stretch">
            <button
              @click="pickImage"
              class="group relative h-40 w-40 shrink-0 rounded-md overflow-hidden bg-[#2a2a2a] border border-dashed border-[#4a4a4a] hover:border-[var(--accent-color)] transition-colors flex items-center justify-center"
              title="Choose a cover image"
            >
              <img v-if="form.cover" :src="form.cover" class="w-full h-full object-cover" alt="" />
              <div
                v-else
                class="flex flex-col items-center gap-2 text-gray-500 group-hover:text-[var(--accent-color)] transition-colors"
              >
                <svg
                  xmlns="http://www.w3.org/2000/svg"
                  width="32"
                  height="32"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                >
                  <line x1="12" y1="5" x2="12" y2="19"></line>
                  <line x1="5" y1="12" x2="19" y2="12"></line>
                </svg>
                <span class="text-[11px]">Add cover</span>
              </div>
              <div
                v-if="form.cover"
                class="absolute inset-0 bg-black/40 opacity-0 group-hover:opacity-100 transition-opacity flex items-center justify-center text-white text-xs"
              >
                Change
              </div>
            </button>

            <div class="flex flex-col gap-3 flex-1 min-w-0">
              <input
                v-model="form.name"
                type="text"
                placeholder="Smart playlist name"
                maxlength="80"
                class="w-full bg-[#2a2a2a] text-white rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)] placeholder-gray-500"
              />
              <textarea
                v-model="form.description"
                placeholder="Description (optional)"
                rows="4"
                maxlength="300"
                class="w-full flex-1 resize-none bg-[#2a2a2a] text-white rounded-md px-3 py-2 text-sm focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)] placeholder-gray-500"
              ></textarea>
              <button
                v-if="form.cover"
                @click="clearCover"
                class="self-start text-[11px] text-gray-500 hover:text-red-400 transition-colors"
              >
                Remove cover
              </button>
            </div>
          </div>
          <input ref="fileInput" type="file" accept="image/*" class="hidden" @change="onFile" />

          <!-- Match selector -->
          <div class="flex items-center gap-2 text-sm text-gray-300">
            <span>Match</span>
            <div class="inline-flex bg-[#2a2a2a] border border-[#2c2c2e] rounded-md p-0.5">
              <button
                @click="form.rules.match = 'all'"
                class="px-3 py-1 rounded-md text-xs font-semibold transition"
                :class="form.rules.match === 'all' ? 'bg-[var(--accent-color)] text-white' : 'text-gray-400 hover:text-white'"
              >all</button>
              <button
                @click="form.rules.match = 'any'"
                class="px-3 py-1 rounded-md text-xs font-semibold transition"
                :class="form.rules.match === 'any' ? 'bg-[var(--accent-color)] text-white' : 'text-gray-400 hover:text-white'"
              >any</button>
            </div>
            <span>of the following rules:</span>
          </div>

          <!-- Condition rows -->
          <div class="space-y-2">
            <div
              v-for="(cond, i) in form.rules.conditions"
              :key="i"
              class="flex items-center gap-2"
            >
              <!-- Field -->
              <select
                v-model="cond.field"
                @change="onFieldChange(cond)"
                class="bg-[#2a2a2a] border border-[#2c2c2e] rounded-md px-2 py-2 text-xs text-white focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)] w-28 shrink-0"
              >
                <option v-for="f in FIELDS" :key="f.key" :value="f.key">{{ f.label }}</option>
              </select>

              <!-- Operator -->
              <select
                v-model="cond.op"
                class="bg-[#2a2a2a] border border-[#2c2c2e] rounded-md px-2 py-2 text-xs text-white focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)] flex-1 min-w-0"
              >
                <option v-for="o in opsFor(cond)" :key="o.op" :value="o.op">{{ o.label }}</option>
              </select>

              <!-- Value -->
              <template v-if="needsValue(cond)">
                <!-- Number / duration / date -->
                <input
                  v-if="fieldType(cond) !== 'text'"
                  v-model="cond.value"
                  type="number"
                  min="0"
                  class="bg-[#2a2a2a] border border-[#2c2c2e] rounded-md px-2 py-2 text-xs text-white focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)] w-28 shrink-0"
                />
                <!-- Text (with library suggestions) -->
                <input
                  v-else
                  v-model="cond.value"
                  type="text"
                  :list="datalistFor(cond)"
                  placeholder="value"
                  class="bg-[#2a2a2a] border border-[#2c2c2e] rounded-md px-2 py-2 text-xs text-white focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)] w-40 shrink-0"
                />
              </template>
              <div v-else class="w-28 shrink-0"></div>

              <!-- Remove -->
              <button
                @click="removeCondition(i)"
                :disabled="form.rules.conditions.length <= 1"
                class="text-gray-500 hover:text-red-400 transition shrink-0 disabled:opacity-30 disabled:hover:text-gray-500"
                title="Remove rule"
              >
                <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                  <line x1="5" y1="12" x2="19" y2="12" />
                </svg>
              </button>
            </div>

            <button
              @click="addCondition"
              class="flex items-center gap-1.5 text-xs font-semibold text-[var(--accent-color)] hover:text-red-400 transition mt-1"
            >
              <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                <line x1="12" y1="5" x2="12" y2="19" /><line x1="5" y1="12" x2="19" y2="12" />
              </svg>
              Add rule
            </button>

            <p v-if="usesGenre && !libraryHasGenre" class="text-[11px] text-amber-400/90 bg-amber-500/10 border border-amber-500/20 rounded-lg px-3 py-2 mt-1">
              None of your tracks have a genre tag yet. Re-scan / reindex your library (Settings) so genre rules can match.
            </p>
          </div>

          <!-- Sort + limit -->
          <div class="flex flex-wrap items-center gap-x-6 gap-y-3 pt-1">
            <div class="flex items-center gap-2">
              <label class="text-xs font-semibold text-gray-400">Sort by</label>
              <select
                v-model="form.sortBy"
                class="bg-[#2a2a2a] border border-[#2c2c2e] rounded-md px-2 py-1.5 text-xs text-white focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)]"
              >
                <option v-for="o in SORT_OPTIONS" :key="o.key" :value="o.key">{{ o.label }}</option>
              </select>
              <div v-if="showSortOrder" class="inline-flex bg-[#2a2a2a] border border-[#2c2c2e] rounded-md p-0.5">
                <button
                  @click="form.sortOrder = 'asc'"
                  class="px-2 py-1 rounded-md text-[11px] font-semibold transition"
                  :class="form.sortOrder === 'asc' ? 'bg-[#2c2c2e] text-white' : 'text-gray-400'"
                >Asc</button>
                <button
                  @click="form.sortOrder = 'desc'"
                  class="px-2 py-1 rounded-md text-[11px] font-semibold transition"
                  :class="form.sortOrder === 'desc' ? 'bg-[#2c2c2e] text-white' : 'text-gray-400'"
                >Desc</button>
              </div>
            </div>

            <label class="flex items-center gap-2 text-xs font-semibold text-gray-400 cursor-pointer">
              <input v-model="limitEnabled" type="checkbox" class="accent-[var(--accent-color)] h-3.5 w-3.5 rounded" />
              Limit to
              <input
                v-model="form.limit"
                type="number"
                min="1"
                :disabled="!limitEnabled"
                class="bg-[#2a2a2a] border border-[#2c2c2e] rounded-md px-2 py-1 text-xs text-white w-16 focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)] disabled:opacity-40"
              />
              songs
            </label>
          </div>
        </div>

        <!-- Footer -->
        <div class="flex items-center justify-between px-6 py-4 border-t border-[#2c2c2e] shrink-0">
          <span class="text-xs text-gray-400">
            <span class="text-white font-semibold">{{ previewCount }}</span> songs match
          </span>
          <div class="flex gap-2.5">
            <button
              @click="cancel"
              class="px-4 py-2 rounded-lg text-sm font-medium text-gray-400 hover:text-white bg-[#2c2c2e] hover:bg-[#3a3a3c] transition"
            >
              Cancel
            </button>
            <button
              @click="save"
              :disabled="!canSave"
              class="px-5 py-2 rounded-lg text-sm font-semibold text-white bg-[var(--accent-color)] hover:bg-red-500 transition shadow-lg disabled:opacity-40 disabled:cursor-not-allowed"
            >
              {{ store.smartModal.mode === 'edit' ? 'Save Changes' : 'Create' }}
            </button>
          </div>
        </div>
        </div>
      </div>
    </Transition>

    <!-- Library suggestions -->
    <datalist id="dl-genre"><option v-for="g in genreOptions" :key="g" :value="g" /></datalist>
    <datalist id="dl-artist"><option v-for="a in artistOptions" :key="a" :value="a" /></datalist>
    <datalist id="dl-album"><option v-for="al in albumOptions" :key="al" :value="al" /></datalist>
  </Teleport>
</template>

<style scoped>
.scrollbar-thin::-webkit-scrollbar {
  width: 8px;
}
</style>
