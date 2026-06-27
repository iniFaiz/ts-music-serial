import { createRouter, createWebHistory } from 'vue-router';
import HomeView from '../views/HomeView.vue';
import SongsView from '../views/SongsView.vue';
import AlbumsView from '../views/AlbumsView.vue';
import AlbumDetail from '../views/AlbumDetail.vue';
import ArtistsView from '../views/ArtistsView.vue';
import ArtistDetail from '../views/ArtistDetail.vue';
import FavoritesView from '../views/FavoritesView.vue';
import PlaylistDetail from '../views/PlaylistDetail.vue';
import PlaylistsView from '../views/PlaylistsView.vue';
import SettingsView from '../views/SettingsView.vue';
import SmartPlaylistDetail from '../views/SmartPlaylistDetail.vue';
import CollectionDetail from '../views/CollectionDetail.vue';

const routes = [
  { path: '/', redirect: '/home' },
  { path: '/home', name: 'Home', component: HomeView },
  { path: '/songs', component: SongsView },
  { path: '/albums', component: AlbumsView },
  { path: '/albums/:name', name: 'AlbumDetail', component: AlbumDetail },
  { path: '/artists', component: ArtistsView },
  { path: '/artists/:name', name: 'ArtistDetail', component: ArtistDetail },
  { path: '/favorites', name: 'Favorites', component: FavoritesView },
  { path: '/playlists', name: 'Playlists', component: PlaylistsView },
  { path: '/playlists/:id', name: 'PlaylistDetail', component: PlaylistDetail },
  { path: '/smart/:id', name: 'SmartPlaylistDetail', component: SmartPlaylistDetail },
  { path: '/collection/:key', name: 'CollectionDetail', component: CollectionDetail },
  { path: '/settings', component: SettingsView },
];

const router = createRouter({
  history: createWebHistory(),
  routes,
});

export default router;
