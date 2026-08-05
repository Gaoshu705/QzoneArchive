<script setup lang="ts">
import { computed } from "vue";
import { storeToRefs } from "pinia";
import { platform } from "@tauri-apps/plugin-os";
import Button from "primevue/button";
import Dialog from "primevue/dialog";
import ProgressSpinner from "primevue/progressspinner";
import { useAuthStore } from "../stores/auth";

const authStore = useAuthStore();
const { dialogVisible, loading, qrImage, status, message, webLoginMode } = storeToRefs(authStore);
const showWebLogin = computed(() => platform() !== "android");
</script>

<template>
  <Dialog
    :visible="dialogVisible"
    modal
    :draggable="false"
    :closable="true"
    class="login-dialog"
    header="登录 QQ 空间"
    @update:visible="(visible) => !visible && authStore.closeLogin()"
  >
    <!-- Web login mode -->
    <div v-if="webLoginMode" class="login-content">
      <div class="qr-frame">
        <ProgressSpinner v-if="loading" stroke-width="4" />
        <i v-else class="pi pi-desktop web-login-icon" />
      </div>

      <div class="login-status" :class="`status-${status}`">
        <span class="status-dot" />
        <p>{{ message }}</p>
      </div>
      <p class="login-help">在新窗口中完成登录后，凭证仅保存在本机 Rust 后端，页面只接收公开用户资料。</p>
      <div class="login-actions">
        <Button
          v-if="status === 'webLoginWaiting' || status === 'webLoginOpened'"
          label="返回扫码登录"
          severity="secondary"
          text
          @click="authStore.cancelWebLogin()"
        />
        <Button
          v-if="status === 'error' || status === 'webLoginCancelled'"
          label="重试网页登录"
          icon="pi pi-refresh"
          :loading="loading"
          @click="authStore.startWebLogin()"
        />
      </div>
    </div>

    <!-- QR login mode -->
    <div v-else class="login-content">
      <div class="qr-frame" :class="{ 'qr-muted': status === 'expired' || status === 'error' }">
        <ProgressSpinner v-if="loading && !qrImage" stroke-width="4" />
        <img v-else-if="qrImage" :src="qrImage" alt="QQ 登录二维码" />
        <i v-else class="pi pi-qrcode" />
        <div v-if="status === 'scanned'" class="qr-confirmed"><i class="pi pi-check" /></div>
      </div>

      <div class="login-status" :class="`status-${status}`">
        <span class="status-dot" />
        <p>{{ message }}</p>
      </div>
      <p class="login-help">二维码登录与凭证由本机 Rust 后端处理，页面只接收二维码和公开用户资料。</p>
      <Button v-if="status === 'expired' || status === 'error'" label="刷新二维码" icon="pi pi-refresh" :loading="loading" @click="authStore.refreshQrCode" />
      <div v-if="showWebLogin" class="login-alt">
        <Button
          label="使用 QQ 账号密码登录"
          icon="pi pi-external-link"
          severity="secondary"
          text
          size="small"
          :disabled="loading"
          @click="authStore.startWebLogin()"
        />
      </div>
    </div>
  </Dialog>
</template>
