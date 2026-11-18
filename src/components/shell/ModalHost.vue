<script setup lang="ts">
/**
 * 弹层宿主（v0.27）：由 URL `?modal=` 驱动。
 * 五个入口各自持 Dialog；本组件只做路由到组件的映射。
 */
import HelpDialog from "../dialogs/HelpDialog.vue";
import SettingsDialog from "../dialogs/SettingsDialog.vue";
import BackupDialog from "../dialogs/BackupDialog.vue";
import ExportDialog from "../dialogs/ExportDialog.vue";
import DuplicatesDialog from "../dialogs/DuplicatesDialog.vue";
import { useModalRoute } from "../../composables/useModalRoute";

const { modal } = useModalRoute();
</script>

<template>
  <HelpDialog v-if="modal === 'help'" />
  <SettingsDialog v-else-if="modal === 'settings'" />
  <BackupDialog v-else-if="modal === 'backup'" />
  <ExportDialog v-else-if="modal === 'export'" />
  <DuplicatesDialog v-else-if="modal === 'duplicates'" />
</template>
