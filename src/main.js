import { createApp } from 'vue';
import './assets/main.css';

// Disable right-click context menu globally (prevents 'Inspect Element')
document.addEventListener('contextmenu', (e) => e.preventDefault());

// Disable web browser shortcuts for devtools and reloading
document.addEventListener('keydown', (e) => {
  const isCtrlShift = e.ctrlKey && e.shiftKey;

  // DevTools: F12, Ctrl+Shift+I, Ctrl+Shift+C, Ctrl+Shift+J
  if (e.key === 'F12' || (isCtrlShift && ['I', 'i', 'C', 'c', 'J', 'j'].includes(e.key))) {
    e.preventDefault();
  }

  // Reloads: F5, Ctrl+R, Ctrl+Shift+R
  if (e.key === 'F5' || (e.ctrlKey && (e.key === 'R' || e.key === 'r'))) {
    e.preventDefault();
  }
});

// Global Vue error sink. Without this, an uncaught error inside a component
// render/watcher/lifecycle hook only lands in the console and the UI silently
// stops updating; logging it (and surfacing a status message on the main app
// once the store is available) makes failures visible.
function installErrorHandler(app, store = null) {
  app.config.errorHandler = (err, instance, info) => {
    console.error(`[Vue error] ${info}`, err);
    try {
      store?.showToast?.(`UI error: ${err.message || err}`, { type: 'error', duration: 6500 });
    } catch {
      // Toasting must never throw from the error handler itself.
    }
  };
}

const windowMode = new URLSearchParams(window.location.search).get('tsWindow');

async function bootstrap() {
  if (windowMode === 'vinyl-scratch') {
    document.documentElement.classList.add('vinyl-native-window');
    const [{ default: VinylScratchApp }, { store: scratchStore }] = await Promise.all([
      import('./VinylScratchApp.vue'),
      import('./store'),
    ]);
    const app = createApp(VinylScratchApp);
    installErrorHandler(app, scratchStore);
    app.mount('#app');
    return;
  }

  const [{ default: App }, { default: router }, { store }] = await Promise.all([
    import('./App.vue'),
    import('./router'),
    import('./store'),
  ]);
  const app = createApp(App);
  installErrorHandler(app, store);
  app.use(router).mount('#app');
}

bootstrap();
