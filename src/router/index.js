import { createRouter, createWebHistory } from 'vue-router';

const routes = [
  { path: '/', redirect: '/home' },
  { path: '/home', name: 'Home', component: () => import('../views/HomeView.vue') },
  { path: '/songs', component: () => import('../views/SongsView.vue') },
  { path: '/albums', component: () => import('../views/AlbumsView.vue') },
  {
    path: '/albums/:name',
    name: 'AlbumDetail',
    component: () => import('../views/AlbumDetail.vue'),
  },
  { path: '/artists', component: () => import('../views/ArtistsView.vue') },
  {
    path: '/artists/:name',
    name: 'ArtistDetail',
    component: () => import('../views/ArtistDetail.vue'),
  },
  {
    path: '/favorites',
    name: 'Favorites',
    component: () => import('../views/FavoritesView.vue'),
  },
  {
    path: '/playlists',
    name: 'Playlists',
    component: () => import('../views/PlaylistsView.vue'),
  },
  {
    path: '/playlists/:id',
    name: 'PlaylistDetail',
    component: () => import('../views/PlaylistDetail.vue'),
  },
  {
    path: '/smart/:id',
    name: 'SmartPlaylistDetail',
    component: () => import('../views/SmartPlaylistDetail.vue'),
  },
  {
    path: '/collection/:key',
    name: 'CollectionDetail',
    component: () => import('../views/CollectionDetail.vue'),
  },
  { path: '/settings', component: () => import('../views/SettingsView.vue') },
];

const router = createRouter({
  history: createWebHistory(),
  routes,
});

export default router;
