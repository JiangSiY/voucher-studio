<script setup lang="ts">
/** 重复核查弹层（v0.27 B4）：全站唯一重复核查入口（工作台待办 / `?modal=duplicates`）。 */
import Dialog from "../ui/Dialog.vue";
import DuplicateReviewPage from "../../pages/DuplicateReviewPage.vue";
import { useModalRoute } from "../../composables/useModalRoute";

const { close } = useModalRoute();
</script>

<template>
  <Dialog :open="true" title="重复核查" wide @close="close">
    <DuplicateReviewPage />
  </Dialog>
</template>
