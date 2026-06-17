<template>
  <section class="bg-[#1c1c1e]/80 border border-white/5 rounded-xl p-5 mb-5">
    <div class="flex items-center gap-2.5 mb-1">
      <slot name="icon" />
      <h2 class="text-base font-semibold text-white">{{ title }}</h2>
    </div>
    <p v-if="description" class="text-xs text-gray-400 mb-2 leading-relaxed">{{ description }}</p>
    <div class="mt-2"><slot /></div>
  </section>
</template>

<script setup>
defineProps({
  title: { type: String, required: true },
  description: { type: String, default: '' },
});
</script>
