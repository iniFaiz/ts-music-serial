<script setup>
import { computed } from 'vue';

defineProps({
  size: { type: Number, default: 24 },
});

// Unique clip-path id per instance so multiple logos on one page don't clash.
let _seq = 0;
const cid = `tslogo-clip-${++_seq}-${Math.random().toString(36).slice(2, 7)}`;
const clipId = computed(() => cid);
</script>

<template>
  <svg
    :width="size"
    :height="size"
    viewBox="0 0 256 256"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    class="shrink-0"
  >
    <g :clip-path="`url(#${clipId})`">
      <rect width="256" height="256" rx="128" fill="white" />
      <circle cx="10.5" cy="134.5" r="201.5" fill="black" />
      <circle cx="321.5" cy="127.5" r="201.5" fill="#FA2D48" />
      <path
        d="M199.5 -67C310.785 -67 401 23.2146 401 134.5C401 245.785 310.785 336 199.5 336C198.665 336 197.832 335.995 197 335.984C307.133 334.645 396 244.951 396 134.5C396 24.0491 307.133 -65.6463 197 -66.9854C197.832 -66.9955 198.665 -67 199.5 -67Z"
        fill="#C9C9C9"
      />
      <path
        d="M376.5 -74C487.785 -74 578 16.2146 578 127.5C578 238.785 487.785 329 376.5 329C265.215 329 175 238.785 175 127.5C175 16.2146 265.215 -74 376.5 -74ZM256 98C239.431 98 226 111.431 226 128C226 144.569 239.431 158 256 158C272.569 158 286 144.569 286 128C286 111.431 272.569 98 256 98Z"
        fill="black"
      />
    </g>
    <defs>
      <clipPath :id="clipId">
        <rect width="256" height="256" rx="128" fill="white" />
      </clipPath>
    </defs>
  </svg>
</template>
