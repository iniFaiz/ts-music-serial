import { reactive } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';

export const store = reactive({
  songs: [],
  loading: false,
  statusMessage: "Ready to scan",
  selectedPath: "",

  // Open folder and scan
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

  // Call Rust to scan
  async scanMusic(path) {
    this.loading = true;
    this.statusMessage = "Scanning...";
    this.songs = [];

    const startTime = performance.now();

    try {
      const result = await invoke("scan_music_folder", { path });
      const endTime = performance.now();
      
      this.songs = result;
      
      const timeSeconds = ((endTime - startTime) / 1000).toFixed(2);
      this.statusMessage = `Found ${result.length} tracks in ${timeSeconds}s`;
    } catch (error) {
      this.statusMessage = `Error: ${error}`;
    } finally {
      this.loading = false;
    }
  }
});