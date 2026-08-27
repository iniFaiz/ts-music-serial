import { createI18n } from 'vue-i18n';
import en from './locales/en';
import id from './locales/id';

const STORAGE_KEY = 'ts-music-language';

export function getInitialLocale() {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved && ['en', 'id'].includes(saved)) {
      return saved;
    }
  } catch {}

  const navLang = navigator.language?.toLowerCase() || '';
  if (navLang.startsWith('id')) {
    return 'id';
  }
  return 'en';
}

export const i18n = createI18n({
  legacy: false,
  locale: getInitialLocale(),
  fallbackLocale: 'en',
  messages: {
    en,
    id,
  },
});

export function setLanguage(lang) {
  if (!['en', 'id'].includes(lang)) return;
  i18n.global.locale.value = lang;
  try {
    localStorage.setItem(STORAGE_KEY, lang);
  } catch {}
  document.documentElement.setAttribute('lang', lang);
}
