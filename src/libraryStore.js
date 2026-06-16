// Minimal IndexedDB-backed key/value store for persisting the music library.
//
// The library used to live in localStorage, which is synchronous and capped at
// roughly 5 MB — a few tens of thousands of tracks is enough to exceed that and
// silently fail to save. IndexedDB has no such practical limit and keeps writes
// off the critical path.

const DB_NAME = 'ts-music';
const STORE_NAME = 'kv';
const DB_VERSION = 1;

let dbPromise = null;

function openDb() {
  if (dbPromise) return dbPromise;
  dbPromise = new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.onupgradeneeded = () => {
      if (!req.result.objectStoreNames.contains(STORE_NAME)) {
        req.result.createObjectStore(STORE_NAME);
      }
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
  return dbPromise;
}

async function withStore(mode, fn) {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_NAME, mode);
    const request = fn(tx.objectStore(STORE_NAME));
    tx.oncomplete = () => resolve(request ? request.result : undefined);
    tx.onerror = () => reject(tx.error);
    tx.onabort = () => reject(tx.error);
  });
}

export function idbGet(key) {
  return withStore('readonly', (store) => store.get(key));
}

export function idbSet(key, value) {
  return withStore('readwrite', (store) => store.put(value, key));
}

export function idbDelete(key) {
  return withStore('readwrite', (store) => store.delete(key));
}
