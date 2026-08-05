import { invoke } from "@tauri-apps/api/core";
import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { cancelWebLogin as cancelWebLoginCommand, checkWebLogin, getQzoneLoginUser, openWebLogin } from "../utils/qlogin";

export interface LoginUser { uin: string; nickname: string; avatarImage?: string }
type Status = "waiting" | "scanned" | "expired" | "refreshed" | "success" | "error" | "loggedOut" | "timedOut" | "cancelled" | "webLoginOpened" | "webLoginWaiting" | "webLoginCancelled";
interface LoginStatus { sessionId?: string; status: Status; message: string; qrImage?: string }
interface QrLoginStart { sessionId: string; qrImage: string }
const delay = (milliseconds: number) => new Promise((resolve) => setTimeout(resolve, milliseconds));
const errorMessage = (error: unknown, fallback: string) => typeof error === "string" ? error : error instanceof Error ? error.message : fallback;

export const useAuthStore = defineStore("auth", () => {
  const dialogVisible = ref(false), loading = ref(false), qrImage = ref("");
  const status = ref<Status>("loggedOut"), message = ref("使用手机 QQ 扫码登录");
  const user = ref<LoginUser>(), webLoginMode = ref(false);
  let pollingRun = 0, qrSessionId: string | undefined;
  const loggedIn = computed(() => status.value === "success");

  async function loadProfile() {
    try { user.value = await getQzoneLoginUser(); }
    catch { user.value = { uin: "", nickname: "QQ 用户" }; message.value = "登录成功，但暂时无法获取用户资料"; }
  }
  async function restoreSession() {
    try {
      const result = await invoke<LoginStatus>("get_login_status"); status.value = result.status; message.value = result.message;
      if (result.status === "success") await loadProfile();
    } catch { status.value = "loggedOut"; }
  }
  async function openLogin() { dialogVisible.value = true; if (!loggedIn.value) await refreshQrCode(); }
  async function cancelQrSession() {
    const id = qrSessionId; qrSessionId = undefined;
    if (id) await invoke("cancel_qr_login", { id }).catch(() => {});
  }
  async function closeLogin() {
    dialogVisible.value = false; pollingRun += 1;
    await Promise.all([cancelQrSession(), webLoginMode.value ? cancelWebLoginCommand().catch(() => {}) : Promise.resolve()]);
    webLoginMode.value = false;
  }
  async function refreshQrCode() {
    const run = ++pollingRun; await cancelQrSession(); loading.value = true; qrImage.value = ""; status.value = "waiting"; message.value = "正在获取登录二维码…";
    try {
      const start = await invoke<QrLoginStart>("start_qr_login"); if (run !== pollingRun) { await invoke("cancel_qr_login", { id: start.sessionId }).catch(() => {}); return; }
      qrSessionId = start.sessionId; qrImage.value = start.qrImage; message.value = "请使用手机 QQ 扫描二维码"; loading.value = false;
      while (run === pollingRun && dialogVisible.value) {
        await delay(1800); if (run !== pollingRun || !dialogVisible.value || !qrSessionId) return;
        const result = await invoke<LoginStatus>("poll_qr_login", { id: qrSessionId }); if (run !== pollingRun) return;
        status.value = result.status; message.value = result.message;
        if (result.status === "refreshed") { if (result.qrImage) qrImage.value = result.qrImage; continue; }
        if (result.status === "success") { qrSessionId = undefined; await loadProfile(); await delay(700); dialogVisible.value = false; pollingRun += 1; return; }
        if (["expired", "error", "timedOut", "cancelled"].includes(result.status)) { qrSessionId = undefined; return; }
      }
    } catch (error) { if (run === pollingRun) { status.value = "error"; message.value = errorMessage(error, "登录服务暂时不可用"); } }
    finally { if (run === pollingRun) loading.value = false; }
  }
  async function startWebLogin() {
    const run = ++pollingRun; await cancelQrSession(); loading.value = true; webLoginMode.value = true; qrImage.value = ""; status.value = "webLoginOpened"; message.value = "正在打开登录窗口…";
    try {
      let result = await openWebLogin(); if (run !== pollingRun) return; status.value = result.status; message.value = result.message; loading.value = false;
      while (run === pollingRun && dialogVisible.value && webLoginMode.value) {
        await delay(2000); if (run !== pollingRun || !dialogVisible.value || !webLoginMode.value) return;
        result = await checkWebLogin(); if (run !== pollingRun) return; status.value = result.status; message.value = result.message;
        if (result.status === "success") { await loadProfile(); await delay(700); dialogVisible.value = false; webLoginMode.value = false; pollingRun += 1; return; }
        if (result.status === "webLoginCancelled" || result.status === "error") { webLoginMode.value = false; return; }
      }
    } catch (error) { if (run === pollingRun) { status.value = "error"; message.value = errorMessage(error, "网页登录服务暂时不可用"); } }
    finally { if (run === pollingRun) loading.value = false; }
  }
  async function cancelWebLogin() { pollingRun += 1; loading.value = false; await cancelWebLoginCommand().catch(() => {}); webLoginMode.value = false; status.value = "loggedOut"; message.value = "使用手机 QQ 扫码登录"; }
  async function logout() {
    pollingRun += 1; loading.value = true; dialogVisible.value = false; qrSessionId = undefined;
    try { await invoke("logout_qzone"); } finally { user.value = undefined; qrImage.value = ""; webLoginMode.value = false; status.value = "loggedOut"; message.value = "尚未登录"; loading.value = false; }
  }
  return { dialogVisible, loading, qrImage, status, message, user, webLoginMode, loggedIn, restoreSession, openLogin, closeLogin, refreshQrCode, startWebLogin, cancelWebLogin, logout };
});
