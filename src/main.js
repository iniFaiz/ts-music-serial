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

const windowMode = new URLSearchParams(window.location.search).get('tsWindow');

async function bootstrap() {
  if (windowMode === 'vinyl-scratch') {
    document.documentElement.classList.add('vinyl-native-window');
    const { default: VinylScratchApp } = await import('./VinylScratchApp.vue');
    createApp(VinylScratchApp).mount('#app');
    return;
  }

  const [{ default: App }, { default: router }] = await Promise.all([
    import('./App.vue'),
    import('./router'),
  ]);
  createApp(App).use(router).mount('#app');
}

bootstrap();
