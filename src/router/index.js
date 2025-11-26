import { createRouter, createWebHistory } from 'vue-router';
import SongsView from '../views/SongsView.vue';
import AlbumsView from '../views/AlbumsView.vue';
import AlbumDetail from '../views/AlbumDetail.vue';
import ArtistsView from '../views/ArtistsView.vue';
import ArtistDetail from '../views/ArtistDetail.vue';

const routes = [
  { path: '/', redirect: '/songs' },
  { path: '/songs', component: SongsView },
  { path: '/albums', component: AlbumsView },
  { path: '/albums/:name', name: 'AlbumDetail', component: AlbumDetail },
  { path: '/artists', component: ArtistsView },
  { path: '/artists/:name', name: 'ArtistDetail', component: ArtistDetail },
];

const router = createRouter({
  history: createWebHistory(),
  routes,
});

export default router;