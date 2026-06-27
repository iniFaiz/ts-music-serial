import { nextTick } from 'vue';

// Runs a route change inside a View Transition so a tagged cover element morphs
// into the matching cover on the destination page (shared-element transition).
// Falls back to a plain navigation when the browser lacks the API.
//
//   navigate  - async fn that performs the route change (e.g. () => router.push(...))
//   sourceEl  - the cover element to morph from (gets the shared name temporarily)
//   name      - the shared view-transition-name (matched on the destination cover)
export async function navigateWithTransition(
  navigate,
  sourceEl = null,
  name = 'shared-cover',
  transitionClass = 'to-artist-transition'
) {
  if (sourceEl && typeof document !== 'undefined') {
    // Clear previous clicked elements to avoid duplicates
    document.querySelectorAll('[data-last-clicked]').forEach((el) => {
      el.removeAttribute('data-last-clicked');
    });
    // Tag the clicked element so we can uniquely find it on back transition
    sourceEl.setAttribute('data-last-clicked', 'true');
  }

  if (typeof document === 'undefined' || !document.startViewTransition) {
    await navigate();
    return;
  }

  if (!sourceEl) {
    try {
      const transition = document.startViewTransition(async () => {
        await navigate();
        await nextTick();
      });
      await transition.finished;
    } catch {
      await navigate();
    }
    return;
  }

  document.documentElement.classList.add(transitionClass);
  const prev = sourceEl.style.getPropertyValue('view-transition-name') || '';
  sourceEl.style.setProperty('view-transition-name', name);

  // Temporarily strip viewTransitionName from any other element on the page
  // to avoid duplicates when we tag the sourceEl.
  const activeTaggedEls = [];
  const allElements = document.querySelectorAll('[style*="view-transition-name"]');
  for (const el of allElements) {
    if (el === sourceEl) continue;
    const vtName = (el.style.getPropertyValue('view-transition-name') || '').trim();
    if (vtName === name || vtName === 'shared-cover') {
      activeTaggedEls.push({ el, prevName: vtName });
      el.style.removeProperty('view-transition-name');
    }
  }

  // Temporarily strip viewTransitionName from any ancestor of sourceEl to allow it to animate independently
  const ancestorTaggedEls = [];
  let curr = sourceEl.parentElement;
  while (curr) {
    const vtName = (curr.style.getPropertyValue('view-transition-name') || '').trim();
    if (vtName) {
      ancestorTaggedEls.push({ el: curr, prevName: vtName });
      curr.style.removeProperty('view-transition-name');
    }
    curr = curr.parentElement;
  }

  try {
    const transition = document.startViewTransition(async () => {
      await navigate();
      // Wait for Vue to flush the new page into the DOM before the API snapshots
      // the destination state.
      await nextTick();
    });
    await transition.finished;
  } finally {
    // Release the name so the list element can't clash on the next capture.
    if (prev) {
      sourceEl.style.setProperty('view-transition-name', prev);
    } else {
      sourceEl.style.removeProperty('view-transition-name');
    }
    // Restore the transition names for the elements we stripped
    for (const item of activeTaggedEls) {
      if (item.prevName) {
        item.el.style.setProperty('view-transition-name', item.prevName);
      }
    }
    // Restore the transition names for the ancestor elements we stripped
    for (const item of ancestorTaggedEls) {
      if (item.prevName) {
        item.el.style.setProperty('view-transition-name', item.prevName);
      }
    }
    document.documentElement.classList.remove(transitionClass);
  }
}

// Find a list cover by its data-cover-key (set on album/artist grid items). Used
// on back-navigation to morph the detail cover into the matching list cover.
// Iterates instead of using an attribute selector so odd characters in names
// (quotes, brackets) can't break the query.
function getRouteKey(route) {
  if (!route || !route.params) return null;
  if (route.name === 'PlaylistDetail' || route.name === 'SmartPlaylistDetail') {
    return route.params.id ?? null;
  }
  if (route.name === 'CollectionDetail') {
    return route.params.key ?? null;
  }
  if (route.name === 'AlbumDetail' || route.name === 'ArtistDetail') {
    return route.params.name ?? null;
  }
  return route.params.name ?? route.params.id ?? null;
}

// Checks if the artist key string contains the target artist name, accounting
// for multiple artists formatted with commas, semicolons, slashes, or ampersands.
function artistKeysMatch(artistKeyStr, targetArtist) {
  if (!artistKeyStr || !targetArtist) return false;
  const lowerTarget = targetArtist.trim().toLowerCase();
  // Split by common delimiters: comma, semicolon, slash, amp, and words like feat/ft
  const parts = artistKeyStr.split(/[,;/&]|\bfeat\b|\bft\b/i).map((s) => s.trim().toLowerCase());
  return parts.includes(lowerTarget) || artistKeyStr.trim().toLowerCase() === lowerTarget;
}

function findCoverByKey(key) {
  if (key == null) return null;
  // Match either a cover key (albums/playlists/collections) or an artist key
  // (song cards/rows) — the latter lets artist back/forward navigation morph
  // even when opened from a page whose items only carry data-artist-key.
  const nodes = document.querySelectorAll('[data-cover-key], [data-artist-key]');
  for (const n of nodes) {
    if (
      n.dataset.coverKey === String(key) ||
      artistKeysMatch(n.dataset.artistKey, String(key))
    ) {
      return n.querySelector('.cover-image') || n;
    }
  }
  return null;
}

// Back-navigation counterpart of navigateWithTransition. The detail cover already
// carries the shared name, so we just need to tag the destination list cover so
// the API morphs the detail art back into its grid slot. Falls back to a plain
// router.back() when the View Transition API is unavailable.
export async function goBackWithTransition(router, name = 'shared-cover') {
  const from = router.currentRoute.value;
  const key = getRouteKey(from);

  if (typeof document === 'undefined' || !document.startViewTransition) {
    router.back();
    return;
  }

  const backPath = window.history.state && window.history.state.back;
  // Pick the morph shape from the cover we're leaving first (an artist cover is
  // circular), then fall back to the destination — so artist back-navigation
  // animates the same way no matter where it was opened from.
  let transitionClass = 'to-album-transition';
  if (from && from.name === 'ArtistDetail') {
    transitionClass = 'to-artist-transition';
  } else if (backPath) {
    try {
      const resolved = router.resolve(backPath);
      if (resolved && (resolved.name === 'ArtistDetail' || resolved.name === 'ArtistsView')) {
        transitionClass = 'to-artist-transition';
      }
    } catch {
      // ignore
    }
  }

  document.documentElement.classList.add(transitionClass);
  let tagged = null;
  const transition = document.startViewTransition(async () => {
    // Wait until the route change actually settles (keep-alive restores the list
    // DOM) before snapshotting, with a timeout so a no-op back can't hang.
    await new Promise((resolve) => {
      const off = router.afterEach(() => {
        off();
        resolve();
      });
      router.back();
      setTimeout(resolve, 500);
    });
    await nextTick();

    // 1. Try to find the exact last clicked cover element
    let el = document.querySelector('[data-last-clicked="true"]');
    if (el) {
      const parent = el.closest('[data-cover-key], [data-artist-key]') || el;
      const coverKey = parent.dataset.coverKey ? String(parent.dataset.coverKey) : '';
      const artistKey = parent.dataset.artistKey ? String(parent.dataset.artistKey) : '';
      const matches =
        coverKey === String(key) ||
        artistKey === String(key) ||
        artistKeysMatch(artistKey, String(key));
      if (!matches) {
        el = null; // Mismatch, clear to fall back to generic finder
      }
    }

    // 2. If no exact match, search using findCoverByKey
    if (!el) {
      el = findCoverByKey(key);
    }

    if (el) {
      tagged = el;
      tagged.dataset._prevVt = el.style.getPropertyValue('view-transition-name') || '';
      el.style.setProperty('view-transition-name', name);
    }
  });

  try {
    await transition.finished;
  } finally {
    if (tagged) {
      if (tagged.dataset._prevVt) {
        tagged.style.setProperty('view-transition-name', tagged.dataset._prevVt);
      } else {
        tagged.style.removeProperty('view-transition-name');
      }
      delete tagged.dataset._prevVt;
    }
    document.documentElement.classList.remove(transitionClass);
  }
}

// Forward-navigation counterpart of goBackWithTransition. Resolves the next route path
// in history to locate the slot cover in the current list, tags it, and performs a
// View Transition as the router navigates forward.
export async function goForwardWithTransition(router, name = 'shared-cover') {
  const forwardPath = window.history.state && window.history.state.forward;
  if (!forwardPath || typeof document === 'undefined' || !document.startViewTransition) {
    router.forward();
    return;
  }

  let transitionClass = 'to-album-transition';
  let key = null;
  try {
    const resolved = router.resolve(forwardPath);
    if (resolved) {
      if (resolved.name === 'ArtistDetail' || resolved.name === 'ArtistsView') {
        transitionClass = 'to-artist-transition';
      }
      key = getRouteKey(resolved);
    }
  } catch {
    // ignore
  }

  document.documentElement.classList.add(transitionClass);
  let tagged = findCoverByKey(key);
  let prevVt = '';
  if (tagged) {
    prevVt = tagged.style.getPropertyValue('view-transition-name') || '';
    tagged.style.setProperty('view-transition-name', name);
  }

  const transition = document.startViewTransition(async () => {
    await new Promise((resolve) => {
      const off = router.afterEach(() => {
        off();
        resolve();
      });
      router.forward();
      setTimeout(resolve, 500);
    });
    await nextTick();
  });

  try {
    await transition.finished;
  } finally {
    if (tagged) {
      if (prevVt) {
        tagged.style.setProperty('view-transition-name', prevVt);
      } else {
        tagged.style.removeProperty('view-transition-name');
      }
    }
    document.documentElement.classList.remove(transitionClass);
  }
}
