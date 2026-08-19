<script setup lang="ts">
import { onMounted } from "vue";
import { useConfigStore } from "./stores/config";
import ToastHost from "./components/ToastHost.vue";

const configStore = useConfigStore();

onMounted(() => {
  configStore.init();
});
</script>

<template>
  <router-view />
  <ToastHost />
</template>
