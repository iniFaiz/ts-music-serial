import { createApp } from 'vue';
import './assets/main.css';
import App from './App.vue';
import router from './router';

// Disable right-click context menu globally (prevents 'Inspect Element')
document.addEventListener('contextmenu', (e) => e.preventDefault());

// Disable web browser shortcuts for devtools and reloading
document.addEventListener('keydown', (e) => {
  const isCtrlShift = e.ctrlKey && e.shiftKey;

  // DevTools: F12, Ctrl+Shift+I, Ctrl+Shift+C, Ctrl+Shift+J
  if (
    e.key === 'F12' ||
    (isCtrlShift && ['I', 'i', 'C', 'c', 'J', 'j'].includes(e.key))
  ) {
    e.preventDefault();
  }

  // Reloads: F5, Ctrl+R, Ctrl+Shift+R
  if (
    e.key === 'F5' ||
    (e.ctrlKey && (e.key === 'R' || e.key === 'r'))
  ) {
    e.preventDefault();
  }
});

createApp(App).use(router).mount('#app');
