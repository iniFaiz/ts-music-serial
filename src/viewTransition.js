import { nextTick } from 'vue';

// Runs a route change inside a View Transition so a tagged cover element morphs
// into the matching cover on the destination page (shared-element transition).
// Falls back to a plain navigation when the browser lacks the API.
//
//   navigate  - async fn that performs the route change (e.g. () => router.push(...))
//   sourceEl  - the cover element to morph from (gets the shared name temporarily)
//   name      - the shared view-transition-name (matched on the destination cover)
export async function navigateWithTransition(navigate, sourceEl, name = 'shared-cover') {
  if (typeof document === 'undefined' || !document.startViewTransition || !sourceEl) {
    await navigate();
    return;
  }

  const prev = sourceEl.style.viewTransitionName;
  sourceEl.style.viewTransitionName = name;

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
  }
}
