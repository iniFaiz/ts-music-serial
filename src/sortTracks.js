// Canonical library ordering, shared by every place that needs sorted tracks:
// artist -> album -> track number -> title.
export function compareTracks(a, b) {
  const artist = (a.artist || 'Unknown Artist')
    .toLowerCase()
    .localeCompare((b.artist || 'Unknown Artist').toLowerCase());
  if (artist !== 0) return artist;

  const album = (a.album || 'Unknown Album')
    .toLowerCase()
    .localeCompare((b.album || 'Unknown Album').toLowerCase());
  if (album !== 0) return album;

  const track = (a.track_number || 0) - (b.track_number || 0);
  if (track !== 0) return track;

  return (a.title || '').toLowerCase().localeCompare((b.title || '').toLowerCase());
}

// Returns a new sorted array, leaving the input untouched.
export function sortTracks(list) {
  return list.slice().sort(compareTracks);
}
