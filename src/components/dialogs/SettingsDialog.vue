<script setup lang="ts">
/** 设置弹层（v0.27 B4）：复用设置页六 Tab，关闭即回原页。 */
import Dialog from "../ui/Dialog.vue";
import SettingsPage from "../../pages/SettingsPage.vue";
import { useModalRoute } from "../../composables/useModalRoute";

const { close } = useModalRoute();
</script>

<template>
  <Dialog :open="true" title="设置" @close="close">
    <SettingsPage />
  </Dialog>
</template>
