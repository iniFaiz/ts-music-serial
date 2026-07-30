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
                <path
                  d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"
                />
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

    <!-- Online metadata (strictly opt-in) -->
    <Section
      title="Online Metadata"
      description="Fill missing tags and album artwork from MusicBrainz and the Cover Art Archive. Existing values are never replaced."
    >
      <ToggleInt
        :modelValue="store.onlineMetadataEnabled"
        @update:modelValue="(v) => store.setOnlineMetadataEnabled(v)"
        label="Import missing metadata online"
      />
      <p class="text-xs text-gray-500 leading-relaxed">
        Turning this on immediately checks your library. Tracks with metadata but no cover receive
        only artwork; tracks with artwork but incomplete tags receive only the missing tags.
      </p>

      <div class="mt-4 pt-4 border-t border-white/5">
        <div class="flex items-center justify-between gap-3">
          <p class="text-xs text-gray-500 leading-relaxed">
            Acoustic fingerprint matching is built in. MusicBrainz completes matched metadata and
            artwork.
          </p>
          <button
            v-if="store.onlineMetadataEnabled"
            @click="store.startOnlineMetadataImport()"
            :disabled="store.onlineMetadataRunning || store.scanCount === 0"
            class="px-3 py-2 bg-[#3a3a3a] hover:bg-[#444] disabled:opacity-40 disabled:cursor-not-allowed text-white text-sm font-medium rounded-md transition-colors shrink-0"
          >
            {{ store.onlineMetadataRunning ? 'Searching...' : 'Scan now' }}
          </button>
        </div>
      </div>

      <div v-if="store.onlineMetadataRunning" class="mt-4">
        <div class="h-1.5 rounded-full bg-white/5 overflow-hidden">
          <div
            class="h-full bg-[var(--accent-color)] transition-all duration-300"
            :style="{
              width: `${
                store.onlineMetadataProgress.total
                  ? (store.onlineMetadataProgress.processed / store.onlineMetadataProgress.total) *
                    100
                  : 0
              }%`,
            }"
          />
        </div>
      </div>
      <p
        v-if="store.onlineMetadataStatus"
        class="text-xs mt-3"
        :class="store.onlineMetadataStatus.includes('error') ? 'text-red-400' : 'text-gray-400'"
      >
        {{ store.onlineMetadataStatus }}
      </p>
    </Section>

    <!-- Audio Output -->
    <Section title="Audio Output" description="Choose which device audio is played through.">
      <SelectInt
        label="Output device"
        :modelValue="store.wasapiExclusive ? '' : store.outputDevice || ''"
        :options="deviceOptions"
        :disabled="store.wasapiExclusive"
        @update:modelValue="onDeviceChange"
      />
      <button
        @click="loadDevices"
        :disabled="store.wasapiExclusive"
        class="text-xs font-medium text-gray-400 hover:text-white mt-1 disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:text-gray-400"
      >
        Refresh devices
      </button>
      <p v-if="store.wasapiExclusive" class="text-xs text-amber-500/80 mt-1">
        Disabled while WASAPI Exclusive Mode is on.
      </p>

      <div class="border-t border-white/5 pt-1 mt-3">
        <ToggleInt
          :modelValue="store.wasapiExclusive"
          @update:modelValue="(v) => store.setWasapiExclusive(v)"
          label="WASAPI Exclusive Mode"
        />
        <p class="text-xs text-gray-500">
          Sends audio straight to the device, bypassing the Windows mixer (no system resampling or
          mixing) and matching the track's lossless bit depth. Other apps can't play sound while
          it's active, and it falls back to shared mode if your device refuses exclusive access.
          Output device selection and crossfade/gapless are disabled in this mode — it always uses
          the system default device.
        </p>
      </div>
    </Section>

    <!-- Playback -->
    <Section title="Playback">
      <SelectInt
        label="Track transition"
        :modelValue="store.wasapiExclusive ? 'off' : store.transitionMode"
        :options="transitionOptions"
        :disabled="store.wasapiExclusive"
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
        :disabled="store.wasapiExclusive"
        @update:modelValue="(v) => store.setCrossfadeSecs(v)"
      />
      <p class="text-xs text-gray-500 -mt-1 mb-2">
        <span v-if="store.wasapiExclusive" class="text-amber-500/80">
          Disabled while WASAPI Exclusive Mode is on.
        </span>
        <span v-else>
          Gapless pre-decodes the next track for seamless transitions. Crossfade overlaps the end of
          one track with the start of the next.
        </span>
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
          Levels loudness across tracks using ReplayGain tags, falling back to an automatic loudness
          analysis (computed once per track in the background).
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

      <div class="border-t border-white/5 pt-1 mt-1">
        <ToggleInt
          :modelValue="store.waveformEnabled"
          @update:modelValue="(v) => store.setWaveformEnabled(v)"
          label="Waveform seek bar"
        />
        <p class="text-xs text-gray-500">
          Replace the seek slider with the track's amplitude waveform. Each track is decoded once to
          build it (cached afterwards), so the first play may take a moment.
        </p>
      </div>
    </Section>

    <!-- Mini Player -->
    <Section
      title="Mini Player"
      description="A compact Apple-Music-style player with synced lyrics. Toggle it any time with Ctrl+Shift+M."
    >
      <ToggleInt
        :modelValue="store.miniAlwaysOnTop"
        @update:modelValue="(v) => store.setMiniAlwaysOnTop(v)"
        label="Always on top"
      />
      <p class="text-xs text-gray-500 -mt-1 mb-3">
        Keeps the mini player floating above other windows while it's open.
      </p>
      <button
        @click="store.enterMiniPlayer()"
        :disabled="store.miniPlayerOpen"
        class="text-sm font-semibold text-[var(--accent-color)] hover:underline disabled:opacity-50"
      >
        Open mini player (Ctrl+Shift+M)
      </button>
    </Section>

    <!-- Equalizer -->
    <Section
      title="Equalizer"
      description="10-band graphic equalizer applied in real time. Boost or cut frequency bands, or pick a preset. Changes take effect instantly, even mid-track."
    >
      <EqualizerPanel />
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
        <label class="text-sm text-gray-300 font-medium block mb-2">
          Musixmatch user token (optional)
          <span v-if="store.musixmatchConfigured" class="text-[var(--accent-color)] text-xs ml-1"
            >✓ configured</span
          >
        </label>
        <div class="flex gap-2">
          <input
            v-model="tokenInput"
            @keyup.enter="saveToken"
            type="password"
            :placeholder="
              store.musixmatchConfigured
                ? '•••••••• (stored securely)'
                : 'Paste your Musixmatch community token'
            "
            class="flex-1 bg-[#2a2a2a] text-sm text-white rounded-md px-3 py-2 focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)] placeholder-gray-600"
          />
          <button
            @click="saveToken"
            class="px-3 py-2 bg-[#3a3a3a] hover:bg-[#444] text-white text-sm rounded-md transition-colors shrink-0"
          >
            Save
          </button>
          <button
            v-if="store.musixmatchConfigured"
            @click="store.setMusixmatchToken('')"
            class="px-3 py-2 text-gray-400 hover:text-white text-sm rounded-md transition-colors shrink-0"
            title="Remove token"
          >
            Clear
          </button>
        </div>
        <p class="text-xs text-gray-500 mt-1">
          Stored securely in your OS credential manager — never saved in the app database.
        </p>
      </div>

      <div v-if="store.lyricsSource !== 'none'" class="border-t border-white/5 mt-3 pt-3">
        <SliderInt
          label="Lyric timing offset"
          :modelValue="store.lyricsOffsetMs"
          :min="-3000"
          :max="3000"
          :step="50"
          suffix=" ms"
          @update:modelValue="(v) => store.setLyricsOffset(v)"
        />
        <p class="text-xs text-gray-500 mt-1">
          Nudge every lyric earlier (positive) or later (negative) if the timing is consistently
          off. 0 = no change.
        </p>
      </div>
    </Section>

    <!-- Discord Rich Presence -->
    <Section
      title="Discord Rich Presence"
      description="Show the track you're playing as your Discord status."
    >
      <ToggleInt
        :modelValue="store.discordEnabled"
        @update:modelValue="(v) => store.setDiscordEnabled(v)"
        label="Enable Discord Rich Presence"
      />
      <p class="text-xs text-gray-500 mt-2 leading-relaxed">
        Shows the artist and track you're listening to on your Discord profile, with the album cover
        as artwork. Pausing hides the status. Requires the Discord desktop app to be running.
      </p>
    </Section>

    <!-- System Tray -->
    <Section title="System Tray" description="Keep music playing when the window is closed.">
      <ToggleInt
        :modelValue="store.closeToTray"
        @update:modelValue="(v) => store.setCloseToTray(v)"
        label="Close to tray"
      />
      <p class="text-xs text-gray-500 mt-2 leading-relaxed">
        While enabled, a tray icon appears and closing the window hides ts-music to the system tray
        instead of quitting — playback keeps running. Click the tray icon to bring the window back,
        or use its menu to control playback and quit.
      </p>
    </Section>

    <!-- Signed application updater -->
    <Section
      title="Software Update"
      :description="
        updaterStatus
          ? `Installed version ${updaterStatus.currentVersion}`
          : 'Check for signed TS Music releases.'
      "
    >
      <p v-if="updaterStatus && !updaterStatus.configured" class="text-xs text-gray-500">
        Updates are disabled in this development build. Official release builds embed the
        verification key and only accept signed packages.
      </p>

      <template v-else>
        <div class="flex items-center justify-between gap-4">
          <div class="min-w-0">
            <h3 class="text-white font-medium text-sm">
              {{
                updateInfo ? `Version ${updateInfo.version} is available` : 'Signed release channel'
              }}
            </h3>
            <p class="text-xs text-gray-500 mt-1">
              {{
                updateInfo
                  ? `Available to ${updateInfo.rolloutPercentage}% of installations.`
                  : updateMessage || 'Updates are verified by Rust before installation.'
              }}
            </p>
          </div>
          <button
            v-if="!updateInfo"
            @click="checkForUpdate"
            :disabled="checkingUpdate || installingUpdate || !updaterStatus"
            class="px-4 py-2 bg-[#3a3a3a] hover:bg-[#444] disabled:opacity-40 disabled:cursor-not-allowed text-white rounded-md transition-colors text-sm font-medium shrink-0"
          >
            {{ checkingUpdate ? 'Checking...' : 'Check for Updates' }}
          </button>
          <button
            v-else
            @click="installUpdate"
            :disabled="installingUpdate"
            class="px-4 py-2 bg-[var(--accent-color)] hover:brightness-110 disabled:opacity-40 disabled:cursor-not-allowed text-white rounded-md transition-colors text-sm font-medium shrink-0"
          >
            {{ installingUpdate ? 'Installing...' : 'Install and Restart' }}
          </button>
        </div>

        <div v-if="installingUpdate" class="mt-4">
          <div class="h-1.5 rounded-full bg-white/5 overflow-hidden">
            <div
              class="h-full bg-[var(--accent-color)] transition-all duration-200"
              :class="{ 'animate-pulse w-1/3': !updateProgress.total }"
              :style="
                updateProgress.total
                  ? {
                      width: `${Math.min(
                        100,
                        (updateProgress.downloaded / updateProgress.total) * 100
                      )}%`,
                    }
                  : undefined
              "
            />
          </div>
          <p class="text-xs text-gray-500 mt-2">
            {{
              updateProgress.finished
                ? 'Signature verified. Starting installer...'
                : updateProgress.total
                  ? `${formatBytes(updateProgress.downloaded)} / ${formatBytes(updateProgress.total)}`
                  : 'Downloading signed update...'
            }}
          </p>
        </div>

        <p
          v-if="updateInfo?.body"
          class="text-xs text-gray-400 leading-relaxed mt-4 whitespace-pre-line"
        >
          {{ updateInfo.body }}
        </p>
        <p v-if="updateError" class="text-xs text-red-400 mt-3">{{ updateError }}</p>
      </template>
    </Section>

    <!-- Keyboard Shortcuts -->
    <Section title="Keyboard Shortcuts" description="Control playback from anywhere in the app.">
      <div class="grid grid-cols-1 sm:grid-cols-2 gap-x-8 gap-y-1.5">
        <div
          v-for="(s, i) in shortcuts"
          :key="i"
          class="flex items-center justify-between gap-3 py-1"
        >
          <span class="text-sm text-gray-300">{{ s.label }}</span>
          <span class="flex items-center gap-1 shrink-0">
            <template v-for="(k, j) in s.keys" :key="j">
              <span v-if="j > 0" class="text-gray-600 text-xs">+</span>
              <kbd
                class="px-1.5 py-0.5 text-xs font-medium text-gray-200 bg-[#2a2a2a] border border-white/10 rounded shadow-sm"
                >{{ k }}</kbd
              >
            </template>
          </span>
        </div>
      </div>
      <p class="text-xs text-gray-500 mt-3">
        Shortcuts are ignored while you're typing in a text field. Hardware media keys (play/pause,
        next, previous) are handled by the system controls.
      </p>
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

    <!-- Sleep timer -->
    <Section
      title="Sleep Timer"
      description="Automatically pause playback after a set time, at the end of the current track, or at the end of the queue."
    >
      <div class="flex flex-wrap gap-2">
        <button
          v-for="opt in sleepQuick"
          :key="opt.value"
          @click="store.setSleepTimer(opt.value)"
          class="px-3 py-1.5 rounded-md text-sm font-medium transition-colors"
          :class="
            isSleepActive(opt.value)
              ? 'bg-[var(--accent-color)] text-white'
              : 'bg-[#2a2a2a] text-gray-300 hover:bg-[#3a3a3a]'
          "
        >
          {{ opt.label }}
        </button>
      </div>
      <div class="flex items-center gap-2 mt-3">
        <input
          v-model.number="customMin"
          @keyup.enter="setCustom"
          type="number"
          min="1"
          max="1440"
          placeholder="Custom minutes"
          class="w-40 bg-[#2a2a2a] text-sm text-white rounded-md px-3 py-2 focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)] placeholder-gray-600"
        />
        <button
          @click="setCustom"
          class="px-3 py-2 bg-[#3a3a3a] hover:bg-[#444] text-white text-sm rounded-md transition-colors shrink-0"
        >
          Set
        </button>
      </div>
      <p v-if="sleepStatus" class="text-xs text-[var(--accent-color)] mt-3 flex items-center gap-2">
        <span>{{ sleepStatus }}</span>
        <button
          @click="store.setSleepTimer('off')"
          class="text-gray-400 hover:text-white underline underline-offset-2"
        >
          Cancel
        </button>
      </p>
    </Section>

    <!-- Library -->
    <Section
      title="Library"
      description="Manage your music library database, playlists, backups, and settings."
    >
      <!-- Import playlist -->
      <div class="flex items-center justify-between gap-4 mb-5">
        <div>
          <h3 class="text-white font-medium text-sm">Import playlist</h3>
          <p class="text-xs text-gray-500">Load an .m3u / .m3u8 file into a new playlist.</p>
        </div>
        <button
          @click="importM3u"
          :disabled="store.loading"
          class="px-4 py-2 bg-[#3a3a3a] hover:bg-[#444] disabled:opacity-40 disabled:cursor-not-allowed text-white rounded-md transition-colors text-sm font-medium shrink-0"
        >
          Import M3U
        </button>
      </div>

      <!-- Export Backup -->
      <div class="flex items-center justify-between gap-4 mb-5 border-t border-white/5 pt-5">
        <div>
          <h3 class="text-white font-medium text-sm">Export Full Backup</h3>
          <p class="text-xs text-gray-500">
            Export a full copy of your library database and settings for moving to another PC.
          </p>
        </div>
        <button
          @click="store.exportBackup()"
          :disabled="store.loading"
          class="px-4 py-2 bg-[#3a3a3a] hover:bg-[#444] disabled:opacity-40 disabled:cursor-not-allowed text-white rounded-md transition-colors text-sm font-medium shrink-0"
        >
          Export Backup
        </button>
      </div>

      <!-- Import Backup -->
      <div class="flex items-center justify-between gap-4 mb-5 border-t border-white/5 pt-5">
        <div>
          <h3 class="text-white font-medium text-sm">Import Backup</h3>
          <p class="text-xs text-gray-500">
            Restore library database and settings from a previously exported backup file.
          </p>
        </div>
        <button
          @click="store.importBackup()"
          :disabled="store.loading"
          class="px-4 py-2 bg-[#3a3a3a] hover:bg-[#444] disabled:opacity-40 disabled:cursor-not-allowed text-white rounded-md transition-colors text-sm font-medium shrink-0"
        >
          Import Backup
        </button>
      </div>

      <!-- Native generated caches -->
      <div class="flex items-center justify-between gap-4 mb-5 border-t border-white/5 pt-5">
        <div>
          <h3 class="text-white font-medium text-sm">Generated cache</h3>
          <p class="text-xs text-gray-500">
            Clear cached covers, waveforms, lyrics and loudness analysis. They are rebuilt on demand.
          </p>
        </div>
        <button
          @click="store.clearNativeCache()"
          :disabled="store.loading"
          class="px-4 py-2 bg-[#3a3a3a] hover:bg-[#444] disabled:opacity-40 disabled:cursor-not-allowed text-white rounded-md transition-colors text-sm font-medium shrink-0"
        >
          Clear Cache
        </button>
      </div>

      <!-- Reset library -->
      <div class="flex items-center justify-between gap-4 border-t border-white/5 pt-5">
        <div>
          <h3 class="text-white font-medium text-sm">Reset library</h3>
          <p class="text-xs text-gray-500">Clear all songs, albums, playlists and likes.</p>
        </div>
        <button
          @click="confirmReset"
          :disabled="store.loading"
          class="px-4 py-2 rounded-md bg-red-600 hover:bg-red-500 disabled:opacity-40 disabled:cursor-not-allowed text-white transition-colors text-sm font-medium shrink-0"
        >
          Reset Library
        </button>
      </div>

      <!-- Footer for status message / loading indicator, same as folders card -->
      <div
        v-if="store.statusMessage"
        class="flex items-center gap-5 mt-5 pt-3 border-t border-white/5"
      >
        <span class="text-xs text-gray-500 truncate">{{ store.statusMessage }}</span>
      </div>
    </Section>

    <!-- Backup Report Modal -->
    <Transition name="modal">
      <div
        v-if="store.showBackupReportModal"
        class="fixed inset-0 z-[300] flex items-center justify-center bg-black/70 backdrop-blur-md"
        @click.self="store.showBackupReportModal = false"
      >
        <div
          class="modal-panel w-[500px] max-w-[92vw] bg-[#1c1c1e] rounded-2xl shadow-2xl border border-[#2c2c2e] p-6 flex flex-col max-h-[80vh]"
        >
          <h2 class="text-lg font-bold text-white mb-3">Backup Import Report</h2>
          <p class="text-sm text-gray-400 mb-4 leading-relaxed">
            The database was imported, but the following songs could not be found on your disk. They
            have been removed from your active library pages to keep your library clean:
          </p>

          <!-- Scrollable list of missing tracks -->
          <div
            class="flex-1 overflow-y-auto min-h-[150px] max-h-[350px] bg-[#2a2a2a] rounded-lg border border-white/5 p-3 mb-6 space-y-2"
          >
            <div
              v-for="(track, idx) in store.missingTracksReport"
              :key="idx"
              class="flex items-center gap-3 text-xs py-1.5 border-b border-white/5 last:border-b-0 text-left"
            >
              <CoverImage :path="track.path" className="h-10 w-10 rounded-md shrink-0" />
              <div class="min-w-0 flex-1">
                <div class="font-semibold text-gray-200 truncate">
                  {{ track.title || 'Untitled' }}
                </div>
                <div class="text-gray-400 truncate">{{ track.artist || 'Unknown Artist' }}</div>
                <div
                  class="text-gray-500 font-mono scale-[0.9] origin-left truncate mt-0.5"
                  :title="track.path"
                >
                  {{ track.path }}
                </div>
              </div>
            </div>
          </div>

          <div class="flex justify-end">
            <button
              @click="store.showBackupReportModal = false"
              class="px-5 py-2 bg-[var(--accent-color)] hover:bg-red-500 text-white rounded-lg text-sm font-semibold transition shadow-lg"
            >
              OK
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, watch } from 'vue';
import { listen } from '@tauri-apps/api/event';
import { store } from '../store';
import { invokeCommand as invoke } from '../generated/ipc';
import Section from '../components/settings/Section.vue';
import ToggleInt from '../components/settings/ToggleInt.vue';
import SelectInt from '../components/settings/SelectInt.vue';
import SliderInt from '../components/settings/SliderInt.vue';
import EqualizerPanel from '../components/EqualizerPanel.vue';
import CoverImage from '../components/CoverImage.vue';

const devices = ref([]);
const updaterStatus = ref(null);
const updateInfo = ref(null);
const updateMessage = ref('');
const updateError = ref('');
const checkingUpdate = ref(false);
const installingUpdate = ref(false);
const updateProgress = ref({ downloaded: 0, total: null, finished: false });
let unlistenUpdaterProgress = null;

const formatBytes = (bytes) => {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB'];
  const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  return `${(bytes / 1024 ** index).toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
};

async function loadUpdaterStatus() {
  try {
    updaterStatus.value = await invoke('updater_status');
  } catch (error) {
    updateError.value = String(error);
  }
}

async function checkForUpdate() {
  checkingUpdate.value = true;
  updateError.value = '';
  updateMessage.value = '';
  try {
    updateInfo.value = await invoke('updater_check');
    if (!updateInfo.value) updateMessage.value = 'You are up to date for the current rollout.';
  } catch (error) {
    updateError.value = String(error);
  } finally {
    checkingUpdate.value = false;
  }
}

async function installUpdate() {
  if (!updateInfo.value || installingUpdate.value) return;
  installingUpdate.value = true;
  updateError.value = '';
  updateProgress.value = { downloaded: 0, total: null, finished: false };
  try {
    await store.flushState();
    await invoke('updater_install', { expectedVersion: updateInfo.value.version });
  } catch (error) {
    updateError.value = String(error);
    installingUpdate.value = false;
  }
}

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

// Reference list mirroring the handler in App.vue (handleKeydown).
const shortcuts = [
  { keys: ['Space'], label: 'Play / pause' },
  { keys: ['Ctrl', '←'], label: 'Previous track' },
  { keys: ['Ctrl', '→'], label: 'Next track' },
  { keys: ['←'], label: 'Seek back 5s' },
  { keys: ['→'], label: 'Seek forward 5s' },
  { keys: ['Shift', '←'], label: 'Seek back 10s' },
  { keys: ['Shift', '→'], label: 'Seek forward 10s' },
  { keys: ['↑'], label: 'Volume up' },
  { keys: ['↓'], label: 'Volume down' },
  { keys: ['0 – 9'], label: 'Jump to 0–90%' },
  { keys: ['Home'], label: 'Restart track' },
  { keys: ['M'], label: 'Mute / unmute' },
  { keys: ['S'], label: 'Shuffle on / off' },
  { keys: ['R'], label: 'Repeat mode' },
  { keys: ['L'], label: 'Like current track' },
  { keys: ['Ctrl', 'K'], label: 'Command palette' },
  { keys: ['Ctrl', 'Shift', 'F'], label: 'Fullscreen player' },
  { keys: ['Ctrl', 'Shift', 'M'], label: 'Mini player' },
];

const lyricsOptions = [
  { value: 'netease', label: 'NetEase (Default)' },
  { value: 'lrclib', label: 'LRCLIB' },
  { value: 'local', label: 'Local (Embedded tag / .lrc file)' },
  { value: 'musixmatch', label: 'Musixmatch' },
  { value: 'none', label: 'Off / Disabled' },
];

// Musixmatch token is write-only from the UI (kept in the OS credential store).
const tokenInput = ref('');
const saveToken = () => {
  store.setMusixmatchToken(tokenInput.value);
  tokenInput.value = '';
};

const sleepQuick = [
  { value: 'off', label: 'Off' },
  { value: 'end', label: 'End of track' },
  { value: 'end-queue', label: 'End of queue' },
  { value: 15, label: '15m' },
  { value: 30, label: '30m' },
  { value: 45, label: '45m' },
  { value: 60, label: '1h' },
];
const customMin = ref(null);
const setCustom = () => {
  const v = Number(customMin.value);
  if (isFinite(v) && v > 0) store.setSleepTimer(v);
};
const isSleepActive = (value) => String(store.sleepTimerMode) === String(value);

// Live countdown for a timed sleep timer.
const now = ref(Date.now());
let sleepTick = null;
const sleepStatus = computed(() => {
  const m = store.sleepTimerMode;
  if (m === 'off') return null;
  if (m === 'end') return 'Stops at the end of the current track';
  if (m === 'end-queue') return 'Stops at the end of the queue';
  const remain = Math.max(0, store.sleepTimerDeadline - now.value);
  const mm = Math.floor(remain / 60000);
  const ss = Math.floor((remain % 60000) / 1000)
    .toString()
    .padStart(2, '0');
  return `Stops in ${mm}:${ss}`;
});

const importM3u = () => store.importPlaylistM3u();

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

function confirmRemoveRoot(root) {
  store.showConfirm({
    title: 'Remove Folder',
    message: `Remove "${root}" and its tracks from the library?`,
    confirmText: 'Remove',
    cancelText: 'Cancel',
    onConfirm: () => {
      store.removeRoot(root);
    },
  });
}

const confirmReset = () => {
  store.showConfirm({
    title: 'Reset Library',
    message: 'Are you sure you want to delete all library data? This cannot be undone.',
    confirmText: 'Reset Library',
    cancelText: 'Cancel',
    onConfirm: () => {
      store.resetLibrary();
    },
  });
};

watch(
  () => store.devicesVersion,
  () => {
    loadDevices();
  }
);

onMounted(async () => {
  loadDevices();
  loadUpdaterStatus();
  unlistenUpdaterProgress = await listen('updater-progress', ({ payload }) => {
    updateProgress.value = {
      downloaded: payload.downloaded,
      total: payload.total ?? updateProgress.value.total,
      finished: payload.finished,
    };
  });
  sleepTick = setInterval(() => {
    now.value = Date.now();
  }, 1000);
});
onUnmounted(() => {
  if (sleepTick) clearInterval(sleepTick);
  if (unlistenUpdaterProgress) unlistenUpdaterProgress();
});
</script>
