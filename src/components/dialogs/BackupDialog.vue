<script setup lang="ts">
/** 备份与恢复弹层（v0.27 B4）：复用备份页，危险操作仍在页内二次确认。 */
import Dialog from "../ui/Dialog.vue";
import RestorePage from "../../pages/RestorePage.vue";
import { useModalRoute } from "../../composables/useModalRoute";

const { close } = useModalRoute();
</script>

<template>
  <Dialog :open="true" title="备份与恢复" narrow @close="close">
    <RestorePage />
  </Dialog>
</template>
