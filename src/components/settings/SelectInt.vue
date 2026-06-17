<template>
  <div class="flex items-center justify-between py-2.5 gap-4">
    <span class="text-gray-300 font-medium text-sm">{{ label }}</span>
    <select
      :value="modelValue"
      @change="$emit('update:modelValue', $event.target.value)"
      class="bg-[#2a2a2a] text-sm text-white rounded-md px-3 py-1.5 focus:outline-none focus:ring-1 focus:ring-[var(--accent-color)] max-w-[65%] truncate"
    >
      <option v-for="opt in options" :key="String(opt.value)" :value="opt.value">
        {{ opt.label }}
      </option>
    </select>
  </div>
</template>

<script setup>
defineProps({
  modelValue: { type: [String, Number], default: '' },
  label: { type: String, required: true },
  // [{ value, label }]
  options: { type: Array, default: () => [] },
});
defineEmits(['update:modelValue']);
</script>
