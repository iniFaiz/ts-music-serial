<script setup>
import { store } from '../store';
import SongList from '../components/SongList.vue';
</script>

<template>
  <div class="h-full overflow-auto">
    <SongList :songs="store.songs" />
  </div>
</template>