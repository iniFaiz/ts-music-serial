import { ref } from 'vue';
import { invokeCommand as invoke } from './generated/ipc';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { store } from './store';
import { invalidateCover } from './coverCache';
import { requestDestructiveConsent } from './destructiveConsent';

// File Information modal + tag editor (edit mode) extracted from SongList.
// Writes tags back into the audio file (Rust/lofty), re-indexes the DB row and
// refreshes every UI copy of the track. Editing the playing track is safe —
// playback decodes from an in-memory copy of the file.
//
// `menu`/`closeMenu` come from the row context menu that opens this modal.
export function useTagEditor({ menu, closeMenu }) {
  const infoModalOpen = ref(false);
  const infoSong = ref(null);
  const infoStat = ref({ playCount: 0, lastPlayed: 0, skipCount: 0 });
  const copyStatus = ref('Copy Path');

  const showFileInfo = async () => {
    infoSong.value = menu.value.song;
    infoModalOpen.value = true;
    closeMenu();
    // Play stats live in the DB now; fetch them for the opened track.
    infoStat.value = infoSong.value
      ? await store.statFor(infoSong.value.path)
      : { playCount: 0, lastPlayed: 0, skipCount: 0 };
  };

  const closeInfoModal = () => {
    if (editSaving.value) return; // don't lose an in-flight save
    infoModalOpen.value = false;
    infoSong.value = null;
    infoStat.value = { playCount: 0, lastPlayed: 0, skipCount: 0 };
    copyStatus.value = 'Copy Path';
    infoEditing.value = false;
    editError.value = '';
  };

  // ---- Tag editor (edit mode of the File Information modal) ----

  const infoEditing = ref(false);
  const editSaving = ref(false);
  const editError = ref('');
  const editForm = ref({ title: '', artist: '', album: '', genre: '', year: '', track_number: '' });
  const editCoverPath = ref(null); // newly picked image (absolute path)
  const editCoverPreview = ref(null); // data-URL thumbnail of the picked image
  const editRemoveCover = ref(false);

  const startEditInfo = () => {
    const s = infoSong.value;
    if (!s) return;
    editForm.value = {
      title: s.title || '',
      // Don't seed the library's display fallbacks into the file's actual tags.
      artist: s.artist === 'Unknown Artist' ? '' : s.artist || '',
      album: s.album === 'Unknown Album' ? '' : s.album || '',
      genre: s.genre || '',
      year: s.year ? String(s.year) : '',
      track_number: s.track_number ? String(s.track_number) : '',
    };
    editCoverPath.value = null;
    editCoverPreview.value = null;
    editRemoveCover.value = false;
    editError.value = '';
    infoEditing.value = true;
  };

  const cancelEditInfo = () => {
    if (editSaving.value) return;
    infoEditing.value = false;
    editError.value = '';
  };

  const pickEditCover = async () => {
    try {
      const sel = await openDialog({
        multiple: false,
        filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp', 'bmp', 'gif'] }],
      });
      if (!sel) return;
      editCoverPath.value = sel;
      editRemoveCover.value = false;
      editCoverPreview.value = await invoke('preview_image', { path: sel }).catch(() => null);
    } catch {
      /* dialog dismissed */
    }
  };

  const removeEditCover = () => {
    editRemoveCover.value = true;
    editCoverPath.value = null;
    editCoverPreview.value = null;
  };

  const saveEditInfo = async () => {
    const s = infoSong.value;
    if (!s || editSaving.value) return;
    editSaving.value = true;
    editError.value = '';
    const yr = parseInt(editForm.value.year, 10);
    const tn = parseInt(editForm.value.track_number, 10);
    try {
      const consentToken = await requestDestructiveConsent('write_track_tags', [s.path]);
      if (!consentToken) {
        editError.value = 'Tag update cancelled';
        return;
      }
      const updated = await invoke('write_track_tags', {
        path: s.path,
        edits: {
          title: editForm.value.title,
          artist: editForm.value.artist,
          album: editForm.value.album,
          genre: editForm.value.genre,
          year: Number.isFinite(yr) && yr > 0 ? yr : null,
          trackNumber: Number.isFinite(tn) && tn > 0 ? tn : null,
        },
        coverPath: editCoverPath.value,
        removeCover: editRemoveCover.value,
        consentToken,
      });
      if (editCoverPath.value || editRemoveCover.value) invalidateCover(updated.path);
      store.applyTrackUpdate(updated);
      infoSong.value = { ...s, ...updated };
      infoEditing.value = false;
      store.statusMessage = `Saved tags: ${updated.title}`;
    } catch (e) {
      editError.value = String(e);
    } finally {
      editSaving.value = false;
    }
  };

  const copyToClipboard = async (text) => {
    try {
      await navigator.clipboard.writeText(text);
      copyStatus.value = 'Copied!';
      setTimeout(() => {
        copyStatus.value = 'Copy Path';
      }, 2000);
    } catch (err) {
      console.error('Failed to copy text:', err);
    }
  };

  return {
    infoModalOpen,
    infoSong,
    infoStat,
    copyStatus,
    showFileInfo,
    closeInfoModal,
    infoEditing,
    editSaving,
    editError,
    editForm,
    editCoverPath,
    editCoverPreview,
    editRemoveCover,
    startEditInfo,
    cancelEditInfo,
    pickEditCover,
    removeEditCover,
    saveEditInfo,
    copyToClipboard,
  };
}
