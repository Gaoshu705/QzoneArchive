<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { storeToRefs } from "pinia";
import Button from "primevue/button";
import Checkbox from "primevue/checkbox";
import ProgressBar from "primevue/progressbar";
import Tag from "primevue/tag";
import {
  cancelFeedArchive, getArchiveProgress, listArchiveSkips, retryArchiveSkip, startFeedArchive,
  type ArchiveProgress, type ArchiveSkipItem,
} from "../utils/qzone";
import { useAuthStore } from "../stores/auth";
import { getArchiveAutoRetry, getArchiveInterval, setArchiveAutoRetry } from "../utils/appSettings";

const authStore = useAuthStore();
const { loggedIn } = storeToRefs(authStore);
const progress = ref<ArchiveProgress>({ status: "idle", pages: 0, fetched: 0, saved: 0, skipped: 0, message: "尚未开始归档" });
const skips = ref<ArchiveSkipItem[]>([]);
const retryingId = ref<number>();
const skipNotice = ref("");
const currentTime = ref(Date.now());
const autoRetryEnabled = ref(getArchiveAutoRetry());
let timer: ReturnType<typeof setInterval> | undefined;
let autoStartTimer: ReturnType<typeof setTimeout> | undefined;
let autoSkipTimer: ReturnType<typeof setTimeout> | undefined;
let autoStarting = false;
let autoRetryingSkips = false;
const AUTO_START_DELAY_MS = 8_000;
const AUTO_SKIP_RETRY_DELAY_MS = 12_000;
const running = computed(() => progress.value.status === "running");
const rateLimited = computed(() => progress.value.status === "limited");
const remainingSeconds = computed(() => Math.max(0, Math.ceil((Number(progress.value.retryAt || 0) * 1000 - currentTime.value) / 1000)));
const rateWaiting = computed(() => rateLimited.value && remainingSeconds.value > 0);
const remainingText = computed(() => `${String(Math.floor(remainingSeconds.value / 60)).padStart(2, "0")}:${String(remainingSeconds.value % 60).padStart(2, "0")}`);
const severity = computed(() => ({ completed: "success", error: "danger", cancelled: "warn", limited: "warn", running: "info", idle: "secondary" }[progress.value.status]));
const statusText = computed(() => ({ idle: "未开始", running: "进行中", completed: "已完成", cancelled: "已取消", limited: "频率保护", error: "失败" }[progress.value.status]));
const pendingSkips = computed(() => skips.value.filter((item) => !item.resolvedAt));
const autoRetryHint = computed(() => {
  if (!autoRetryEnabled.value) return "关闭后失败和异常请求需要手动继续";
  if (running.value) return "归档运行中，异常列表会在空闲后继续重试";
  if (rateWaiting.value) return `频率保护中，等待 ${remainingText.value} 后继续`;
  if (pendingSkips.value.length) return `将持续重试 ${pendingSkips.value.length} 条待处理异常`;
  if (progress.value.status === "error") return "检测到失败后将自动继续归档";
  return "失败后自动继续，待重试异常会按顺序处理";
});

function clearAutoTimers() {
  clearTimeout(autoStartTimer);
  clearTimeout(autoSkipTimer);
  autoStartTimer = undefined;
  autoSkipTimer = undefined;
}

function scheduleAutoWork() {
  if (!autoRetryEnabled.value || !loggedIn.value) {
    clearAutoTimers();
    return;
  }
  if (running.value || autoStarting) return;

  if ((progress.value.status === "error" || (rateLimited.value && !rateWaiting.value)) && !autoStartTimer) {
    autoStartTimer = setTimeout(() => {
      autoStartTimer = undefined;
      void autoStartArchive();
    }, AUTO_START_DELAY_MS);
    return;
  }

  if (!rateWaiting.value && retryingId.value === undefined && pendingSkips.value.length > 0 && !autoSkipTimer && !autoRetryingSkips) {
    autoSkipTimer = setTimeout(() => {
      autoSkipTimer = undefined;
      void retryPendingSkips();
    }, AUTO_SKIP_RETRY_DELAY_MS);
  }
}

async function autoStartArchive() {
  if (!loggedIn.value || running.value || rateWaiting.value) return;
  autoStarting = true;
  try {
    await start();
  } finally {
    autoStarting = false;
    scheduleAutoWork();
  }
}

async function retryPendingSkips() {
  if (!loggedIn.value || running.value || retryingId.value !== undefined || autoRetryingSkips) return;
  autoRetryingSkips = true;
  try {
    for (const item of pendingSkips.value) {
      if (running.value || rateWaiting.value) break;
      await retrySkip(item);
      await new Promise((resolve) => setTimeout(resolve, AUTO_SKIP_RETRY_DELAY_MS));
    }
  } finally {
    autoRetryingSkips = false;
    scheduleAutoWork();
  }
}

async function refresh() {
  try { progress.value = await getArchiveProgress(); } catch { /* 保留当前状态 */ }
  if (!loggedIn.value) { skips.value = []; return; }
  try { skips.value = await listArchiveSkips(); } catch { /* 保留当前列表 */ }
  scheduleAutoWork();
}
function beginPolling() { clearInterval(timer); timer = setInterval(() => { currentTime.value = Date.now(); void refresh(); }, 600); }
watch(autoRetryEnabled, (enabled) => {
  setArchiveAutoRetry(enabled);
  if (enabled) {
    beginPolling();
    scheduleAutoWork();
  } else {
    clearAutoTimers();
  }
});
async function start() {
  if (!loggedIn.value) return;
  beginPolling();
  try { progress.value = await startFeedArchive(getArchiveInterval()); }
  catch { await refresh(); }
  finally {
    await refresh();
    if (progress.value.status === "limited" || (autoRetryEnabled.value && (progress.value.status === "error" || pendingSkips.value.length))) beginPolling();
    else { clearInterval(timer); timer = undefined; }
  }
}
async function cancel() { await cancelFeedArchive(); await refresh(); }
async function retrySkip(item: ArchiveSkipItem) {
  retryingId.value = item.id;
  skipNotice.value = "";
  try {
    const result = await retryArchiveSkip(item.id);
    skipNotice.value = result.message;
  } catch (error) {
    skipNotice.value = String(error);
  } finally {
    retryingId.value = undefined;
    await refresh();
  }
}
function formatTime(timestamp?: number) {
  return timestamp ? new Date(timestamp * 1000).toLocaleString("zh-CN", { hour12: false }) : "—";
}
function offsetLabel(item: ArchiveSkipItem) {
  if (item.offsetAdvance <= 0) return `${item.cursorOffset}（待定位）`;
  const end = item.cursorOffset + item.offsetAdvance - 1;
  return end > item.cursorOffset ? `${item.cursorOffset}–${end}` : String(item.cursorOffset);
}
onMounted(async () => {
  await refresh();
  currentTime.value = Date.now();
  if (running.value || rateLimited.value || (autoRetryEnabled.value && (progress.value.status === "error" || pendingSkips.value.length))) beginPolling();
});
onBeforeUnmount(() => { clearInterval(timer); clearAutoTimers(); });
</script>

<template>
  <section class="surface-card task-card">
    <div class="section-heading"><div><p class="section-kicker">ARCHIVE JOB</p><h3>QQ 空间动态归档</h3></div><Tag :value="statusText" :severity="severity" /></div>
    <p class="task-message">{{ progress.message }}</p>
    <ProgressBar v-if="running" mode="indeterminate" style="height: 7px" />
    <div v-if="rateLimited" class="task-rate-limit"><span><i class="pi pi-shield" /></span><div><strong>接口频率保护</strong><p>为防止接口请求过于频繁，每 10 分钟最多请求 300 页。归档进度已保存，{{ rateWaiting ? `等待 ${remainingText} 后可继续` : "现在可以继续归档" }}。</p></div><b v-if="rateWaiting">{{ remainingText }}</b></div>
    <div class="task-stats"><div><span>已读取页数</span><strong>{{ progress.pages }}</strong></div><div><span>接口记录</span><strong>{{ progress.fetched }}</strong></div><div><span>写入记录</span><strong>{{ progress.saved }}</strong></div><div><span>待重试异常</span><strong>{{ progress.skipped }}</strong></div></div>
    <div v-if="!loggedIn" class="task-login-notice"><span><i class="pi pi-lock" /></span><div><strong>请先登录 QQ 空间</strong><p>登录后才能创建或继续归档任务。</p></div><Button label="立即登录" icon="pi pi-sign-in" size="small" @click="authStore.openLogin" /></div>
    <label class="task-auto-retry" for="archive-auto-retry">
      <Checkbox v-model="autoRetryEnabled" binary input-id="archive-auto-retry" />
      <span><strong>自动重试</strong><small>{{ autoRetryHint }}</small></span>
    </label>
    <div class="task-actions"><Button :label="running ? '归档中…' : rateWaiting ? `请等待 ${remainingText}` : rateLimited ? '继续归档' : '开始归档'" icon="pi pi-download" :disabled="running || rateWaiting || !loggedIn" @click="start" /><Button v-if="running" label="取消" icon="pi pi-times" severity="secondary" outlined @click="cancel" /></div>
  </section>

  <section v-if="skips.length" class="surface-card task-skips">
    <div class="task-skips-heading"><div><span><i class="pi pi-exclamation-triangle" /></span><div><p class="section-kicker">SKIPPED REQUESTS</p><h3>异常跳过列表</h3></div></div><small>异常位置不会阻塞后续归档，可逐条重试。</small></div>
    <p v-if="skipNotice" class="task-skip-notice"><i class="pi pi-info-circle" />{{ skipNotice }}</p>
    <div class="task-skip-list">
      <article v-for="item in skips" :key="item.id" class="task-skip-item" :class="{ 'is-resolved': item.resolvedAt }">
        <div class="task-skip-state"><span><i :class="item.resolvedAt ? 'pi pi-check' : 'pi pi-forward'" /></span></div>
        <div class="task-skip-copy">
          <div><strong>第 {{ item.pageNumber }} 页 · offset {{ offsetLabel(item) }}</strong><Tag :value="item.resolvedAt ? '已恢复' : '待重试'" :severity="item.resolvedAt ? 'success' : 'warn'" /></div>
          <p>{{ item.error }}</p>
          <small>跳过于 {{ formatTime(item.skippedAt) }}<template v-if="item.retryCount"> · 已重试 {{ item.retryCount }} 次 · 最近 {{ formatTime(item.lastRetryAt) }}</template><template v-if="item.resolvedAt"> · 恢复 {{ item.recoveredRecords }} 条</template></small>
        </div>
        <Button :label="retryingId === item.id ? '重试中…' : item.resolvedAt ? '已恢复' : '单独重试'" icon="pi pi-refresh" size="small" outlined :loading="retryingId === item.id" :disabled="running || Boolean(item.resolvedAt) || retryingId !== undefined" @click="retrySkip(item)" />
      </article>
    </div>
  </section>

  <section class="surface-card task-tips">
    <div class="task-tips-heading"><span><i class="pi pi-info-circle" /></span><h4>温馨提示</h4></div>
    <ul>
      <li>空间内容的获取基于 QQ 空间的<strong>互动列表</strong>来获取。没有被点赞或评论过的动态无法被恢复。</li>
      <li>出现<strong>频繁提示</strong>时建议换个时间再继续。程序支持<strong>断点续传</strong>，可以接着上次的进度继续归档。</li>
      <li>归档过程中<strong>不要切换 QQ 客户端账号</strong>，否则可能有冻结风险。</li>
    </ul>
  </section>
</template>
