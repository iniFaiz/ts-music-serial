import { nextTick } from 'vue';
import { useRouter } from 'vue-router';

// Per-route scroll restoration for the app's shared scroll container,
// extracted from App.vue. Registers its own router guards — call during
// component setup.
//
// * beforeEach: saves the leaving route's vertical position and every
//   `.shelf-row` horizontal offset, and records history-back intent (Vue
//   Router has already moved the history entry by then, whose `forward` route
//   is the page being left) so detail pages can restore their previous
//   reading position instead of starting at the top.
// * afterEach: restores after the DOM settles; detail pages start at the top
//   unless it is a genuine back navigation.
//
// Both maps are capped (insertion-ordered evict-oldest) so an unbounded
// session of visits cannot grow them forever.
export function useScrollRestoration({ getContainer }) {
  const SCROLL_CACHE_LIMIT = 120;
  const scrollPositions = new Map();
  const horizontalScrollPositions = new Map();
  let restoreScrollOnBackTo = null;

  function rememberScrollPosition(map, key, value) {
    map.set(key, value);
    if (map.size > SCROLL_CACHE_LIMIT) {
      map.delete(map.keys().next().value);
    }
  }

  const router = useRouter();

  router.beforeEach((to, from) => {
    const containerEl = getContainer();
    const historyState = window.history.state;
    const isHistoryBack =
      historyState?.current === to.fullPath && historyState?.forward === from.fullPath;
    restoreScrollOnBackTo = isHistoryBack ? to.fullPath : null;

    if (containerEl) {
      const container =
        containerEl.querySelector('.overflow-auto') || containerEl;
      rememberScrollPosition(scrollPositions, from.fullPath, container.scrollTop);

      // Save horizontal scroll positions
      const horizontalShelves = containerEl.querySelectorAll('.shelf-row');
      const horizPos = [];
      horizontalShelves.forEach((el) => {
        const section = el.closest('section');
        const titleEl = section ? section.querySelector('h2') : null;
        const title = titleEl ? titleEl.textContent.trim() : '';
        horizPos.push({
          title,
          scrollLeft: el.scrollLeft,
        });
      });
      rememberScrollPosition(horizontalScrollPositions, from.fullPath, horizPos);
    }
  });

  router.afterEach((to) => {
    const shouldRestoreScroll = restoreScrollOnBackTo === to.fullPath;
    restoreScrollOnBackTo = null;

    nextTick(() => {
      const containerEl = getContainer();
      if (!containerEl) return;
      const container =
        containerEl.querySelector('.overflow-auto') || containerEl;

      // Detail pages should always start scrolled to the top
      const isDetailPage = [
        'AlbumDetail',
        'ArtistDetail',
        'PlaylistDetail',
        'SmartPlaylistDetail',
        'CollectionDetail',
      ].includes(to.name);
      const pos = isDetailPage && !shouldRestoreScroll ? 0 : scrollPositions.get(to.fullPath) || 0;

      const originalBehavior = container.style.scrollBehavior;
      container.style.scrollBehavior = 'auto';
      container.scrollTop = pos;
      container.style.scrollBehavior = originalBehavior;

      // Restore horizontal scroll positions if not a detail page
      if (!isDetailPage) {
        const horizPos = horizontalScrollPositions.get(to.fullPath);
        if (horizPos && horizPos.length > 0) {
          nextTick(() => {
            const horizontalShelves = containerEl.querySelectorAll('.shelf-row');
            horizontalShelves.forEach((el, index) => {
              const section = el.closest('section');
              const titleEl = section ? section.querySelector('h2') : null;
              const title = titleEl ? titleEl.textContent.trim() : '';
              const match = horizPos.find((p) => p.title === title) || horizPos[index];
              if (match) {
                const orig = el.style.scrollBehavior;
                el.style.scrollBehavior = 'auto';
                el.scrollLeft = match.scrollLeft;
                el.style.scrollBehavior = orig;
              }
            });
          });
        }
      }
    });
  });
}
