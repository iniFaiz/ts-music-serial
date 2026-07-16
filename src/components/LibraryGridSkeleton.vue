<script setup>
defineProps({
  count: { type: Number, default: 6 },
  round: { type: Boolean, default: false },
  label: { type: String, default: 'Loading library' },
});
</script>

<template>
  <div
    class="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 2xl:grid-cols-6 gap-x-6 gap-y-10"
    role="status"
    aria-live="polite"
    :aria-label="label"
  >
    <div
      v-for="index in count"
      :key="index"
      class="animate-pulse"
      :class="{ 'text-center': round }"
    >
      <div
        class="w-full aspect-square mb-3 bg-[#282828]"
        :class="round ? 'rounded-full mx-auto max-w-[200px]' : 'rounded-md'"
      ></div>
      <div class="h-3 bg-[#2f2f2f] rounded" :class="round ? 'w-1/2 mx-auto' : 'w-3/4'"></div>
      <div v-if="!round" class="h-3 w-1/2 bg-[#282828] rounded mt-2"></div>
    </div>
    <span class="sr-only">{{ label }}…</span>
  </div>
</template>
