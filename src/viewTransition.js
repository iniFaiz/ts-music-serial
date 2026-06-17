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
  sourceEl,
  name = 'shared-cover',
  transitionClass = 'to-artist-transition'
) {
  if (typeof document === 'undefined' || !document.startViewTransition || !sourceEl) {
    await navigate();
    return;
  }

  document.documentElement.classList.add(transitionClass);
  const prev = sourceEl.style.viewTransitionName;
  sourceEl.style.viewTransitionName = name;

  // Temporarily strip viewTransitionName from any other element on the page
  // to avoid duplicates when we tag the sourceEl.
  const activeTaggedEls = [];
  const allElements = document.querySelectorAll('*');
  for (const el of allElements) {
    if (
      el !== sourceEl &&
      (el.style.viewTransitionName === name || el.style.viewTransitionName === 'shared-cover')
    ) {
      activeTaggedEls.push({ el, prevName: el.style.viewTransitionName });
      el.style.viewTransitionName = '';
    }
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
    sourceEl.style.viewTransitionName = prev;
    // Restore the transition names for the elements we stripped
    for (const item of activeTaggedEls) {
      item.el.style.viewTransitionName = item.prevName;
    }
    document.documentElement.classList.remove(transitionClass);
  }
}

// Find a list cover by its data-cover-key (set on album/artist grid items). Used
// on back-navigation to morph the detail cover into the matching list cover.
// Iterates instead of using an attribute selector so odd characters in names
// (quotes, brackets) can't break the query.
function findCoverByKey(key) {
  if (key == null) return null;
  const nodes = document.querySelectorAll('[data-cover-key]');
  for (const n of nodes) {
    if (n.dataset.coverKey === String(key)) {
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
  const key = (from && from.params && (from.params.name ?? from.params.id)) ?? null;

  if (typeof document === 'undefined' || !document.startViewTransition) {
    router.back();
    return;
  }

  const transitionClass =
    from && from.name === 'ArtistDetail' ? 'to-artist-transition' : 'to-album-transition';
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
    const el = findCoverByKey(key);
    if (el) {
      tagged = el;
      tagged.dataset._prevVt = el.style.viewTransitionName || '';
      el.style.viewTransitionName = name;
    }
  });

  try {
    await transition.finished;
  } finally {
    if (tagged) {
      tagged.style.viewTransitionName = tagged.dataset._prevVt || '';
      delete tagged.dataset._prevVt;
    }
    document.documentElement.classList.remove(transitionClass);
  }
}
