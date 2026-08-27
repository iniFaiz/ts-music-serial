<template>
  <div class="p-6 max-w-2xl mx-auto pb-16">
    <h1 class="text-2xl font-bold text-white mb-6">{{ $t('settings.title') }}</h1>

    <!-- Language -->
    <Section
      :title="$t('settings.language.title')"
      :description="$t('settings.language.description')"
    >
      <SelectInt
        :modelValue="store.language"
        @update:modelValue="(v) => store.setLanguage(v)"
        :label="$t('settings.language.label')"
        :options="[
          { value: 'en', label: $t('settings.language.en') },
          { value: 'id', label: $t('settings.language.id') },
        ]"
      />
    </Section>

    <!-- Music Folders -->
    <Section
      :title="$t('settings.musicFolders.title')"
      :description="$t('settings.musicFolders.description')"
    >
      <button
        @click="store.selectAndScan()"
        :disabled="store.loading"
        class="text-sm font-semibold text-[var(--accent-color)] hover:underline disabled:opacity-50"
      >
        {{ $t('settings.musicFolders.addFolder') }}
      </button>

      <div class="mt-4">
        <div class="text-xs uppercase tracking-wider text-gray-500 mb-2">{{ $t('settings.musicFolders.addedFolders') }}</div>
        <div v-if="store.roots.length === 0" class="text-sm text-gray-500 py-2">
          {{ $t('settings.musicFolders.noFolders') }}
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
              {{ $t('settings.musicFolders.remove') }}
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
          {{ $t('settings.musicFolders.refresh') }}
        </button>
        <button
          @click="store.reindexLibrary()"
          :disabled="store.loading || store.roots.length === 0"
          class="text-sm font-medium text-gray-300 hover:text-white disabled:opacity-40"
        >
          {{ $t('settings.musicFolders.reindex') }}
        </button>
        <span class="text-xs text-gray-500 truncate">{{ store.statusMessage }}</span>
      </div>
    </Section>

    <!-- Online metadata (strictly opt-in) -->
    <Section
      :title="$t('settings.onlineMetadata.title')"
      :description="$t('settings.onlineMetadata.description')"
    >
      <ToggleInt
        :modelValue="store.onlineMetadataEnabled"
        @update:modelValue="(v) => store.setOnlineMetadataEnabled(v)"
        :label="$t('settings.onlineMetadata.label')"
      />
      <p class="text-xs text-gray-500 leading-relaxed">
        {{ $t('settings.onlineMetadata.hint') }}
      </p>

      <div class="mt-4 pt-4 border-t border-white/5">
        <div class="flex items-center justify-between gap-3">
          <p class="text-xs text-gray-500 leading-relaxed">
            {{ $t('settings.onlineMetadata.fingerprintHint') }}
          </p>
          <button
            v-if="store.onlineMetadataEnabled"
            @click="store.startOnlineMetadataImport()"
            :disabled="store.onlineMetadataRunning || store.scanCount === 0"
            class="px-3 py-2 bg-[#3a3a3a] hover:bg-[#444] disabled:opacity-40 disabled:cursor-not-allowed text-white text-sm font-medium rounded-md transition-colors shrink-0"
          >
            {{ store.onlineMetadataRunning ? $t('settings.onlineMetadata.searching') : $t('settings.onlineMetadata.scanNow') }}
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
    <Section :title="$t('settings.audioOutput.title')" :description="$t('settings.audioOutput.description')">
      <SelectInt
        :label="$t('settings.audioOutput.label')"
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
        {{ $t('settings.audioOutput.refresh') }}
      </button>
      <p v-if="store.wasapiExclusive" class="text-xs text-amber-500/80 mt-1">
        {{ $t('settings.audioOutput.wasapiDisabledHint') }}
      </p>

      <div class="border-t border-white/5 pt-1 mt-3">
        <ToggleInt
          :modelValue="store.wasapiExclusive"
          @update:modelValue="(v) => store.setWasapiExclusive(v)"
          :label="$t('settings.audioOutput.wasapiExclusiveLabel')"
        />
        <p class="text-xs text-gray-500">
          {{ $t('settings.audioOutput.wasapiExclusiveHint') }}
        </p>
      </div>
    </Section>

    <!-- Playback -->
    <Section :title="$t('settings.playback.title')">
      <SelectInt
        :label="$t('settings.playback.transitionLabel')"
        :modelValue="store.wasapiExclusive ? 'off' : store.transitionMode"
        :options="transitionOptions"
        :disabled="store.wasapiExclusive"
        @update:modelValue="(v) => store.setTransitionMode(v)"
      />
      <SliderInt
        v-if="store.transitionMode === 'crossfade'"
        :label="$t('settings.playback.crossfadeDuration')"
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
          {{ $t('settings.audioOutput.wasapiDisabledHint') }}
        </span>
        <span v-else>
          {{ $t('settings.playback.crossfadeHint') }}
        </span>
      </p>

      <div class="border-t border-white/5 pt-1">
        <ToggleInt
          :modelValue="store.normalizationEnabled"
          @update:modelValue="(v) => store.setNormalizationEnabled(v)"
          :label="$t('settings.playback.normalizationLabel')"
        />
        <SliderInt
          v-if="store.normalizationEnabled"
          :label="$t('settings.playback.preampLabel')"
          :modelValue="store.normalizationPreampDb"
          :min="-12"
          :max="12"
          :step="1"
          suffix=" dB"
          @update:modelValue="(v) => store.setNormalizationPreamp(v)"
        />
        <p class="text-xs text-gray-500">
          {{ $t('settings.playback.normalizationHint') }}
        </p>
      </div>

      <div class="border-t border-white/5 pt-1 mt-1">
        <ToggleInt
          :modelValue="store.visualizerEnabled"
          @update:modelValue="(v) => store.setVisualizerEnabled(v)"
          :label="$t('settings.playback.visualizerLabel')"
        />
        <p class="text-xs text-gray-500">
          {{ $t('settings.playback.visualizerHint') }}
        </p>
      </div>

      <div class="border-t border-white/5 pt-1 mt-1">
        <ToggleInt
          :modelValue="store.waveformEnabled"
          @update:modelValue="(v) => store.setWaveformEnabled(v)"
          :label="$t('settings.playback.waveformLabel')"
        />
        <p class="text-xs text-gray-500">
          {{ $t('settings.playback.waveformHint') }}
        </p>
      </div>
    </Section>

    <!-- Mini Player -->
    <Section
      :title="$t('settings.miniPlayer.title')"
      :description="$t('settings.miniPlayer.description')"
    >
      <ToggleInt
        :modelValue="store.miniAlwaysOnTop"
        @update:modelValue="(v) => store.setMiniAlwaysOnTop(v)"
        :label="$t('settings.miniPlayer.alwaysOnTop')"
      />
      <p class="text-xs text-gray-500 -mt-1 mb-3">
        {{ $t('settings.miniPlayer.alwaysOnTopHint') }}
      </p>
      <button
        @click="store.enterMiniPlayer()"
        :disabled="store.miniPlayerOpen"
        class="text-sm font-semibold text-[var(--accent-color)] hover:underline disabled:opacity-50"
      >
        {{ $t('settings.miniPlayer.openButton') }}
      </button>
    </Section>

    <!-- Equalizer -->
    <Section
      :title="$t('settings.equalizer.title')"
      :description="$t('settings.equalizer.description')"
    >
      <EqualizerPanel />
    </Section>

    <!-- Lyrics -->
    <Section
      :title="$t('settings.lyrics.title')"
      :description="$t('settings.lyrics.description')"
    >
      <SelectInt
        :label="$t('settings.lyrics.sourceLabel')"
        :modelValue="store.lyricsSource"
        :options="lyricsOptions"
        @update:modelValue="(v) => store.setLyricsSource(v)"
      />
      <div v-if="store.lyricsSource === 'musixmatch'" class="mt-3">
        <label for="musixmatch-user-token" class="text-sm text-gray-300 font-medium block mb-2">
          {{ $t('settings.lyrics.musixmatchToken') }}
          <span v-if="store.musixmatchConfigured" class="text-[var(--accent-color)] text-xs ml-1"
            >{{ $t('settings.lyrics.configured') }}</span
          >
        </label>
        <div class="flex gap-2">
          <input
            id="musixmatch-user-token"
            :aria-label="$t('settings.lyrics.musixmatchToken')"
            v-model="tokenInput"
            @keyup.enter="saveToken"
            type="password"
            :placeholder="
              store.musixmatchConfigured
                ? $t('settings.lyrics.tokenPlaceholderConfigured')
                : $t('settings.lyrics.tokenPlaceholderEmpty')
            "
            class="flex-1 bg-[#2a2a2a] text-sm text-white rounded-md px-3 py-2 focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)] placeholder-gray-600"
          />
          <button
            @click="saveToken"
            class="px-3 py-2 bg-[#3a3a3a] hover:bg-[#444] text-white text-sm rounded-md transition-colors shrink-0"
          >
            {{ $t('common.save') }}
          </button>
          <button
            v-if="store.musixmatchConfigured"
            @click="store.setMusixmatchToken('')"
            class="px-3 py-2 text-gray-400 hover:text-white text-sm rounded-md transition-colors shrink-0"
            title="Remove token"
          >
            {{ $t('common.clear') }}
          </button>
        </div>
        <p class="text-xs text-gray-500 mt-1">
          {{ $t('settings.lyrics.tokenSecurityHint') }}
        </p>
      </div>

      <div v-if="store.lyricsSource !== 'none'" class="border-t border-white/5 mt-3 pt-3">
        <SliderInt
          :label="$t('settings.lyrics.offsetLabel')"
          :modelValue="store.lyricsOffsetMs"
          :min="-3000"
          :max="3000"
          :step="50"
          suffix=" ms"
          @update:modelValue="(v) => store.setLyricsOffset(v)"
        />
        <p class="text-xs text-gray-500 mt-1">
          {{ $t('settings.lyrics.offsetHint') }}
        </p>
      </div>
    </Section>

    <!-- Discord Rich Presence -->
    <Section
      :title="$t('settings.discord.title')"
      :description="$t('settings.discord.description')"
    >
      <ToggleInt
        :modelValue="store.discordEnabled"
        @update:modelValue="(v) => store.setDiscordEnabled(v)"
        :label="$t('settings.discord.label')"
      />
      <p class="text-xs text-gray-500 mt-2 leading-relaxed">
        {{ $t('settings.discord.hint') }}
      </p>
    </Section>

    <!-- System Tray -->
    <Section :title="$t('settings.systemTray.title')" :description="$t('settings.systemTray.description')">
      <ToggleInt
        :modelValue="store.closeToTray"
        @update:modelValue="(v) => store.setCloseToTray(v)"
        :label="$t('settings.systemTray.label')"
      />
      <p class="text-xs text-gray-500 mt-2 leading-relaxed">
        {{ $t('settings.systemTray.hint') }}
      </p>
    </Section>

    <!-- Signed application updater -->
    <Section
      :title="$t('settings.updater.title')"
      :description="
        updaterStatus
          ? $t('settings.updater.installedVersion', { version: updaterStatus.currentVersion })
          : $t('settings.updater.checkReleases')
      "
    >
      <p v-if="updaterStatus && !updaterStatus.configured" class="text-xs text-gray-500">
        {{ $t('settings.updater.devBuildHint') }}
      </p>

      <template v-else>
        <div class="flex items-center justify-between gap-4">
          <div class="min-w-0">
            <h3 class="text-white font-medium text-sm">
              {{
                updateInfo ? $t('settings.updater.versionAvailable', { version: updateInfo.version }) : $t('settings.updater.signedChannel')
              }}
            </h3>
            <p class="text-xs text-gray-500 mt-1">
              {{
                updateInfo
                  ? $t('settings.updater.rolloutHint', { percent: updateInfo.rolloutPercentage })
                  : updateMessage || $t('settings.updater.verifiedHint')
              }}
            </p>
          </div>
          <button
            v-if="!updateInfo"
            @click="checkForUpdate"
            :disabled="checkingUpdate || installingUpdate || !updaterStatus"
            class="px-4 py-2 bg-[#3a3a3a] hover:bg-[#444] disabled:opacity-40 disabled:cursor-not-allowed text-white rounded-md transition-colors text-sm font-medium shrink-0"
          >
            {{ checkingUpdate ? $t('settings.updater.checking') : $t('settings.updater.checkButton') }}
          </button>
          <button
            v-else
            @click="installUpdate"
            :disabled="installingUpdate"
            class="px-4 py-2 bg-[var(--accent-color)] hover:brightness-110 disabled:opacity-40 disabled:cursor-not-allowed text-white rounded-md transition-colors text-sm font-medium shrink-0"
          >
            {{ installingUpdate ? $t('settings.updater.installing') : $t('settings.updater.installButton') }}
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
                ? $t('settings.updater.verifiedStarting')
                : updateProgress.total
                  ? `${formatBytes(updateProgress.downloaded)} / ${formatBytes(updateProgress.total)}`
                  : $t('settings.updater.downloading')
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
    <Section :title="$t('settings.shortcuts.title')" :description="$t('settings.shortcuts.description')">
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
    <Section :title="$t('settings.performance.title')">
      <ToggleInt
        :modelValue="store.useParallelism"
        @update:modelValue="(v) => store.setParallelism(v)"
        :label="$t('settings.performance.parallelismLabel')"
      />
      <p class="text-xs text-gray-500">
        {{ $t('settings.performance.parallelismHint') }}
      </p>
    </Section>

    <!-- Sleep timer -->
    <Section
      :title="$t('settings.sleepTimer.title')"
      :description="$t('settings.sleepTimer.description')"
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
          :aria-label="$t('settings.sleepTimer.customPlaceholder')"
          :placeholder="$t('settings.sleepTimer.customPlaceholder')"
          class="w-40 bg-[#2a2a2a] text-sm text-white rounded-md px-3 py-2 focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)] placeholder-gray-600"
        />
        <button
          @click="setCustom"
          class="px-3 py-2 bg-[#3a3a3a] hover:bg-[#444] text-white text-sm rounded-md transition-colors shrink-0"
        >
          {{ $t('settings.sleepTimer.setButton') }}
        </button>
      </div>
      <p v-if="sleepStatus" class="text-xs text-[var(--accent-color)] mt-3 flex items-center gap-2">
        <span>{{ sleepStatus }}</span>
        <button
          @click="store.setSleepTimer('off')"
          class="text-gray-400 hover:text-white underline underline-offset-2"
        >
          {{ $t('settings.sleepTimer.cancel') }}
        </button>
      </p>
    </Section>

    <!-- Library -->
    <Section
      :title="$t('settings.library.title')"
      :description="$t('settings.library.description')"
    >
      <!-- Import playlist -->
      <div class="flex items-center justify-between gap-4 mb-5">
        <div>
          <h3 class="text-white font-medium text-sm">{{ $t('settings.library.importPlaylist') }}</h3>
          <p class="text-xs text-gray-500">{{ $t('settings.library.importPlaylistDesc') }}</p>
        </div>
        <button
          @click="importM3u"
          :disabled="store.loading"
          class="px-4 py-2 bg-[#3a3a3a] hover:bg-[#444] disabled:opacity-40 disabled:cursor-not-allowed text-white rounded-md transition-colors text-sm font-medium shrink-0"
        >
          {{ $t('settings.library.importM3u') }}
        </button>
      </div>

      <!-- Export Backup -->
      <div class="flex items-center justify-between gap-4 mb-5 border-t border-white/5 pt-5">
        <div>
          <h3 class="text-white font-medium text-sm">{{ $t('settings.library.exportBackup') }}</h3>
          <p class="text-xs text-gray-500">
            {{ $t('settings.library.exportBackupDesc') }}
          </p>
        </div>
        <button
          @click="store.exportBackup()"
          :disabled="store.loading"
          class="px-4 py-2 bg-[#3a3a3a] hover:bg-[#444] disabled:opacity-40 disabled:cursor-not-allowed text-white rounded-md transition-colors text-sm font-medium shrink-0"
        >
          {{ $t('settings.library.exportBackupButton') }}
        </button>
      </div>

      <!-- Import Backup -->
      <div class="flex items-center justify-between gap-4 mb-5 border-t border-white/5 pt-5">
        <div>
          <h3 class="text-white font-medium text-sm">{{ $t('settings.library.importBackup') }}</h3>
          <p class="text-xs text-gray-500">
            {{ $t('settings.library.importBackupDesc') }}
          </p>
        </div>
        <button
          @click="store.importBackup()"
          :disabled="store.loading"
          class="px-4 py-2 bg-[#3a3a3a] hover:bg-[#444] disabled:opacity-40 disabled:cursor-not-allowed text-white rounded-md transition-colors text-sm font-medium shrink-0"
        >
          {{ $t('settings.library.importBackupButton') }}
        </button>
      </div>

      <!-- Native generated caches -->
      <div class="flex items-center justify-between gap-4 mb-5 border-t border-white/5 pt-5">
        <div>
          <h3 class="text-white font-medium text-sm">{{ $t('settings.library.generatedCache') }}</h3>
          <p class="text-xs text-gray-500">
            {{ $t('settings.library.generatedCacheDesc') }}
          </p>
        </div>
        <button
          @click="store.clearNativeCache()"
          :disabled="store.loading"
          class="px-4 py-2 bg-[#3a3a3a] hover:bg-[#444] disabled:opacity-40 disabled:cursor-not-allowed text-white rounded-md transition-colors text-sm font-medium shrink-0"
        >
          {{ $t('settings.library.clearCache') }}
        </button>
      </div>

      <!-- Reset library -->
      <div class="flex items-center justify-between gap-4 border-t border-white/5 pt-5">
        <div>
          <h3 class="text-white font-medium text-sm">{{ $t('settings.library.resetLibrary') }}</h3>
          <p class="text-xs text-gray-500">{{ $t('settings.library.resetLibraryDesc') }}</p>
        </div>
        <button
          @click="confirmReset"
          :disabled="store.loading"
          class="px-4 py-2 rounded-md bg-red-600 hover:bg-red-500 disabled:opacity-40 disabled:cursor-not-allowed text-white transition-colors text-sm font-medium shrink-0"
        >
          {{ $t('settings.library.resetLibraryButton') }}
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

    <!-- App Credit & Info Section -->
    <AppCredit :version="updaterStatus?.currentVersion || '1.0.0'" />

    <!-- Backup Report Modal -->
    <Transition name="modal">
      <div
        v-if="store.showBackupReportModal"
        class="fixed inset-0 z-[300] flex items-center justify-center"
      >
        <button
          type="button"
          class="fixed inset-0 bg-black/70 backdrop-blur-md cursor-default border-0 w-full h-full"
          tabindex="-1"
          aria-label="Close dialog"
          @click="store.showBackupReportModal = false"
        ></button>
        <div
          ref="backupReportModalRef"
          role="dialog"
          aria-modal="true"
          aria-labelledby="backup-report-title"
          class="modal-panel relative z-10 w-[500px] max-w-[92vw] bg-[#1c1c1e] rounded-2xl shadow-2xl border border-[#2c2c2e] p-6 flex flex-col max-h-[80vh]"
        >
          <h2 id="backup-report-title" class="text-lg font-bold text-white mb-3">{{ $t('settings.backupReport.title') }}</h2>
          <p class="text-sm text-gray-400 mb-4 leading-relaxed">
            {{ $t('settings.backupReport.description') }}
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
              {{ $t('settings.backupReport.ok') }}
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { listen } from '@tauri-apps/api/event';
import { store } from '../store';
import { invokeCommand as invoke } from '../generated/ipc';
import Section from '../components/settings/Section.vue';
import ToggleInt from '../components/settings/ToggleInt.vue';
import SelectInt from '../components/settings/SelectInt.vue';
import SliderInt from '../components/settings/SliderInt.vue';
import EqualizerPanel from '../components/EqualizerPanel.vue';
import CoverImage from '../components/CoverImage.vue';
import AppCredit from '../components/settings/AppCredit.vue';
import { useFocusTrap } from '../useFocusTrap';

const { t } = useI18n();

const backupReportModalRef = ref(null);
useFocusTrap(backupReportModalRef, () => store.showBackupReportModal, {
  onEscape: () => {
    store.showBackupReportModal = false;
  },
});

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
  { value: '', label: t('settings.audioOutput.defaultDevice') },
  ...devices.value.map((d) => ({
    value: d.name,
    label: d.is_default ? `${d.name} (${t('settings.audioOutput.defaultDevice')})` : d.name,
  })),
]);

const transitionOptions = computed(() => [
  { value: 'off', label: t('settings.playback.transitionOff') },
  { value: 'gapless', label: t('settings.playback.transitionGapless') },
  { value: 'crossfade', label: t('settings.playback.transitionCrossfade') },
]);

// Human-readable shortcut reference for the handler in src/useGlobalShortcuts.js
// (handleKeydown). Kept in sync manually — update both when changing shortcuts.
const shortcuts = computed(() => [
  { keys: ['Space', 'K'], label: t('settings.shortcuts.labels.playPause') },
  { keys: ['Ctrl', '←'], label: t('settings.shortcuts.labels.prevTrack') },
  { keys: ['Ctrl', '→'], label: t('settings.shortcuts.labels.nextTrack') },
  { keys: ['←'], label: t('settings.shortcuts.labels.seekBackward') },
  { keys: ['→'], label: t('settings.shortcuts.labels.seekForward') },
  { keys: ['Shift', '←'], label: `${t('settings.shortcuts.labels.seekBackward')} (10s)` },
  { keys: ['Shift', '→'], label: `${t('settings.shortcuts.labels.seekForward')} (10s)` },
  { keys: ['↑'], label: t('settings.shortcuts.labels.volumeUp') },
  { keys: ['↓'], label: t('settings.shortcuts.labels.volumeDown') },
  { keys: ['Shift', '↑'], label: `${t('settings.shortcuts.labels.volumeUp')} (x2)` },
  { keys: ['Shift', '↓'], label: `${t('settings.shortcuts.labels.volumeDown')} (x2)` },
  { keys: ['0 – 9'], label: 'Jump 0–90%' },
  { keys: ['Home'], label: 'Restart' },
  { keys: ['M'], label: t('settings.shortcuts.labels.toggleMute') },
  { keys: ['S'], label: t('common.shuffle') },
  { keys: ['R'], label: t('common.repeat') },
  { keys: ['L'], label: t('player.addToFavorites') },
  { keys: ['Ctrl', 'K'], label: t('settings.shortcuts.labels.commandPalette') },
  { keys: ['Esc'], label: t('common.close') },
  { keys: ['Ctrl', 'Shift', 'F'], label: t('player.fullScreen') },
  { keys: ['Ctrl', 'Shift', 'M'], label: t('settings.shortcuts.labels.miniPlayer') },
]);

const lyricsOptions = computed(() => [
  { value: 'netease', label: t('settings.lyrics.sources.netease') },
  { value: 'lrclib', label: t('settings.lyrics.sources.all') },
  { value: 'local', label: t('settings.lyrics.sources.local') },
  { value: 'musixmatch', label: t('settings.lyrics.sources.musixmatch') },
  { value: 'none', label: t('settings.lyrics.sources.none') },
]);

// Musixmatch token is write-only from the UI (kept in the OS credential store).
const tokenInput = ref('');
const saveToken = () => {
  store.setMusixmatchToken(tokenInput.value);
  tokenInput.value = '';
};

const sleepQuick = computed(() => [
  { value: 'off', label: t('settings.sleepTimer.off') },
  { value: 'end', label: t('settings.sleepTimer.endOfTrack') },
  { value: 'end-queue', label: t('settings.sleepTimer.endOfQueue') },
  { value: 15, label: t('settings.sleepTimer.min15') },
  { value: 30, label: t('settings.sleepTimer.min30') },
  { value: 45, label: t('settings.sleepTimer.min45') },
  { value: 60, label: t('settings.sleepTimer.min60') },
]);
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
    title: t('settings.library.confirmRemoveTitle'),
    message: t('settings.library.confirmRemoveMessage', { root }),
    confirmText: t('common.remove'),
    cancelText: t('common.cancel'),
    onConfirm: () => {
      store.removeRoot(root);
    },
  });
}

const confirmReset = () => {
  store.showConfirm({
    title: t('settings.library.confirmResetTitle'),
    message: t('settings.library.confirmResetMessage'),
    confirmText: t('settings.library.resetLibraryButton'),
    cancelText: t('common.cancel'),
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
