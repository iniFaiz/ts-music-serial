import { reactive } from 'vue';
import { createStoreActions, createStoreState } from './store/modules';

const storeTarget = createStoreState();
Object.defineProperties(storeTarget, Object.getOwnPropertyDescriptors(createStoreActions()));

export const store = reactive(storeTarget);

// Start restoring the native library as soon as the main store is constructed.
// Views intentionally mount against `libraryReady === false` and render their
// loading state until this promise finishes.
store.loadLibrary();
