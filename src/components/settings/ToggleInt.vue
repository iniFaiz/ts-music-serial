<template>
  <div class="flex items-center justify-between py-2.5">
    <span class="text-gray-300 font-medium text-sm">{{ label }}</span>
    <button
      @click="toggle"
      class="relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none shrink-0"
      :class="modelValue ? 'bg-[var(--accent-color)]' : 'bg-gray-700'"
    >
      <span
        class="inline-block h-4 w-4 transform rounded-full bg-white transition-transform"
        :class="modelValue ? 'translate-x-6' : 'translate-x-1'"
      />
    </button>
  </div>
</template>

<script setup>
const props = defineProps({
  modelValue: {
    type: Boolean,
    required: true,
  },
  label: {
    type: String,
    required: true,
  },
});

const emit = defineEmits(['update:modelValue']);

function toggle() {
  emit('update:modelValue', !props.modelValue);
}
</script>
