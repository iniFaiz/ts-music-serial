import { reactive } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';

export const store = reactive({
  // State
  songs: [],
  loading: false,
  statusMessage: "Ready to scan",
  selectedPath: "",
  searchQuery: "",
  useParallelism: true,
  scanComplete: false,
  scanDuration: "0",
  scanCount: 0,

  // Player State
  currentSong: null,
  isPlaying: false,
  volume: 1.0,
  currentTime: 0,
  duration: 0,
  queue: [],
  shuffleMode: false,
  
  // Actions
  loadLibrary() {
    const saved = localStorage.getItem('music_library');
    if (saved) {
      try {
        this.songs = JSON.parse(saved);
        
        // Ensure sorted on load as well
        this.songs.sort((a, b) => {
          const artistA = (a.artist || "Unknown Artist").toLowerCase();
          const artistB = (b.artist || "Unknown Artist").toLowerCase();
          if (artistA < artistB) return -1;
          if (artistA > artistB) return 1;

          const albumA = (a.album || "Unknown Album").toLowerCase();
          const albumB = (b.album || "Unknown Album").toLowerCase();
          if (albumA < albumB) return -1;
          if (albumA > albumB) return 1;

          const trackA = a.track_number || 0;
          const trackB = b.track_number || 0;
          if (trackA < trackB) return -1;
          if (trackA > trackB) return 1;

          const titleA = (a.title || "").toLowerCase();
          const titleB = (b.title || "").toLowerCase();
          if (titleA < titleB) return -1;
          if (titleA > titleB) return 1;

          return 0;
        });

        this.scanCount = this.songs.length;
        this.statusMessage = `Loaded ${this.songs.length} songs from storage`;
        this.scanComplete = true;
      } catch (e) {
        console.error("Failed to load library", e);
      }
    }
  },

  resetLibrary() {
    this.songs = [];
    this.scanCount = 0;
    this.currentSong = null;
    this.queue = [];
    this.isPlaying = false;
    localStorage.removeItem('music_library');
    this.statusMessage = "Library reset";
    this.scanComplete = false;
  },

  async selectAndScan() {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        recursive: true,
      });

      if (selected) {
        this.selectedPath = selected;
        await this.scanMusic(selected);
      }
    } catch (err) {
      console.error(err);
      this.statusMessage = "Error opening dialog";
    }
  },

  // Call rust ro scan
  async scanMusic(path) {
    this.loading = true;
    this.scanComplete = false;
    this.statusMessage = "Scanning...";
    // Don't clear songs here, we want to append
    // this.songs = []; 

    const startTime = performance.now();

    try {
      const result = await invoke("scan_music_folder", { 
        path, 
        useParallelism: this.useParallelism 
      });
      const endTime = performance.now();
      
      // Merge with existing songs, avoiding duplicates by path
      const existingPaths = new Set(this.songs.map(s => s.path));
      const newSongs = result.filter(s => !existingPaths.has(s.path));
      const combinedSongs = [...this.songs, ...newSongs];

      // Sort songs: Artist -> Album -> Track -> Title
      combinedSongs.sort((a, b) => {
        const artistA = (a.artist || "Unknown Artist").toLowerCase();
        const artistB = (b.artist || "Unknown Artist").toLowerCase();
        if (artistA < artistB) return -1;
        if (artistA > artistB) return 1;

        const albumA = (a.album || "Unknown Album").toLowerCase();
        const albumB = (b.album || "Unknown Album").toLowerCase();
        if (albumA < albumB) return -1;
        if (albumA > albumB) return 1;

        const trackA = a.track_number || 0;
        const trackB = b.track_number || 0;
        if (trackA < trackB) return -1;
        if (trackA > trackB) return 1;

        const titleA = (a.title || "").toLowerCase();
        const titleB = (b.title || "").toLowerCase();
        if (titleA < titleB) return -1;
        if (titleA > titleB) return 1;

        return 0;
      });

      this.songs = combinedSongs;
      localStorage.setItem('music_library', JSON.stringify(this.songs));
      
      const timeSeconds = ((endTime - startTime) / 1000).toFixed(2);
      this.statusMessage = `Added ${newSongs.length} new tracks in ${timeSeconds}s`;
      
      this.scanDuration = timeSeconds;
      this.scanCount = this.songs.length;
      this.scanComplete = true;
    } catch (error) {
      this.statusMessage = `Error: ${error}`;
    } finally {
      this.loading = false;
    }
  },

  closePopup() {
    this.scanComplete = false;
  },

  // Player Actions
  playSong(song, newQueue = null) {
    if (newQueue) {
      this.queue = [...newQueue];
    }
    this.currentSong = song;
    this.isPlaying = true;
  },

  togglePlay() {
    this.isPlaying = !this.isPlaying;
  },

  nextSong() {
    if (!this.currentSong || this.queue.length === 0) return;
    
    let nextIndex;
    if (this.shuffleMode) {
      nextIndex = Math.floor(Math.random() * this.queue.length);
    } else {
      const currentIndex = this.queue.findIndex(s => s.path === this.currentSong.path);
      nextIndex = currentIndex + 1;
      if (nextIndex >= this.queue.length) nextIndex = 0; // Loop back to start
    }
    
    this.currentSong = this.queue[nextIndex];
    this.isPlaying = true;
  },

  prevSong() {
    if (!this.currentSong || this.queue.length === 0) return;
    
    // If more than 3 seconds in, restart song
    if (this.currentTime > 3) {
      this.currentTime = 0;
      // We need a way to signal the audio element to seek to 0. 
      // Since store is reactive, we can just rely on the component watching 'currentSong' 
      // but for seeking within same song we might need a trigger.
      // For now, let's just change song logic.
      return; 
    }

    let prevIndex;
    if (this.shuffleMode) {
      prevIndex = Math.floor(Math.random() * this.queue.length);
    } else {
      const currentIndex = this.queue.findIndex(s => s.path === this.currentSong.path);
      prevIndex = currentIndex - 1;
      if (prevIndex < 0) prevIndex = this.queue.length - 1;
    }
    
    this.currentSong = this.queue[prevIndex];
    this.isPlaying = true;
  },

  toggleShuffle() {
    this.shuffleMode = !this.shuffleMode;
  },

  get filteredSongs() {
    if (!this.searchQuery) return this.songs;
    
    const lower = this.searchQuery.toLowerCase();
    return this.songs.filter(song => 
      (song.title && song.title.toLowerCase().includes(lower)) ||
      (song.artist && song.artist.toLowerCase().includes(lower)) ||
      (song.album && song.album.toLowerCase().includes(lower))
    );
  }
});

store.loadLibrary();