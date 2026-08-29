<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { fetch } from "@tauri-apps/plugin-http";
import { save } from "@tauri-apps/plugin-dialog";
import { storeToRefs } from "pinia";
import { useRouter } from "vue-router";
import Button from "primevue/button";
import ProgressBar from "primevue/progressbar";
import Tag from "primevue/tag";
import QzoneText from "../components/QzoneText.vue";
import StatCard from "../components/StatCard.vue";
import { useAuthStore } from "../stores/auth";
import { exportAllArchivedZip, getArchiveOverview, getArchiveProgress, getInteractionRanking, listArchivedFeeds, type ArchiveItem, type ArchiveOverview, type ArchiveProgress, type InteractionRank } from "../utils/qzone";

const router = useRouter();
const authStore = useAuthStore();
const { loggedIn, user } = storeToRefs(authStore);
const overview = ref<ArchiveOverview>({ dynamics: 0, pictures: 0, comments: 0, likes: 0, databaseBytes: 0 });
const progress = ref<ArchiveProgress>({ status: "idle", pages: 0, fetched: 0, saved: 0, skipped: 0, message: "尚未开始归档" });
const recent = ref<ArchiveItem[]>([]);
const ranking = ref<InteractionRank[]>([]);
const loading = ref(false);
const exporting = ref(false);
const exportMessage = ref("");
const exportError = ref("");
const avatarSources = reactive<Record<string, string>>({});
const storageText = computed(() => overview.value.databaseBytes < 1024 * 1024 ? `${(overview.value.databaseBytes / 1024).toFixed(1)} KB` : `${(overview.value.databaseBytes / 1024 / 1024).toFixed(1)} MB`);
const taskSeverity = computed(() => ({ completed: "success", running: "info", error: "danger", cancelled: "warn", limited: "warn", idle: "secondary" }[progress.value.status]));
const taskText = computed(() => ({ completed: "已完成", running: "归档中", error: "异常", cancelled: "已取消", limited: "频率保护", idle: "未开始" }[progress.value.status]));
const formatTime = (seconds: number) => seconds ? new Intl.DateTimeFormat("zh-CN", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }).format(new Date(seconds * 1000)) : "时间未知";
const avatarUrl = (uin: string) => `https://qlogo2.store.qq.com/qzone/${uin}/${uin}/50?${Date.now()}`;
const rankingMax = computed(() => Math.max(1, ...ranking.value.map((item) => item.interactions)));

function releaseAvatars() {
  Object.values(avatarSources).forEach((url) => URL.revokeObjectURL(url));
  Object.keys(avatarSources).forEach((uin) => delete avatarSources[uin]);
}
async function loadAvatar(uin: string) {
  if (!uin || avatarSources[uin]) return;
  try {
    const response = await fetch(avatarUrl(uin), {
      method: "GET",
      headers: { Accept: "image/avif,image/webp,image/png,image/jpeg,image/*,*/*;q=0.8", Referer: "https://user.qzone.qq.com/" },
    });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const contentType = response.headers.get("content-type") || "image/jpeg";
    if (!contentType.startsWith("image/")) throw new Error(`无效头像类型 ${contentType}`);
    avatarSources[uin] = URL.createObjectURL(new Blob([await response.arrayBuffer()], { type: contentType }));
  } catch (reason) { console.warn(`QQ ${uin} 头像加载失败`, reason); }
}

async function loadDashboard() {
  releaseAvatars();
  if (!loggedIn.value) { overview.value = { dynamics: 0, pictures: 0, comments: 0, likes: 0, databaseBytes: 0 }; recent.value = []; ranking.value = []; return; }
  loading.value = true;
  try {
    const [stats, items, task, ranks] = await Promise.all([getArchiveOverview(), listArchivedFeeds(3, 0), getArchiveProgress(), getInteractionRanking(8)]);
    overview.value = stats; recent.value = items; progress.value = task; ranking.value = ranks;
    const avatarUins = [...items.flatMap((item) => item.authorUin ? [item.authorUin] : []), ...ranks.map((item) => item.uin)];
    await Promise.all([...new Set(avatarUins)].map(loadAvatar));
  } finally { loading.value = false; }
}
function primaryAction() { loggedIn.value ? router.push("/tasks") : authStore.openLogin(); }
async function exportAll() {
  if (exporting.value || !loggedIn.value || !overview.value.dynamics) return;
  exporting.value = true;
  exportMessage.value = "";
  exportError.value = "";
  try {
    const date = new Date().toISOString().slice(0, 10);
    const path = await save({
      defaultPath: `QQ空间完整离线归档-${date}.zip`,
      filters: [{ name: "ZIP 离线归档", extensions: ["zip"] }],
    });
    if (!path) return;
    const result = await exportAllArchivedZip(path);
    exportMessage.value = `完整离线归档已导出：${result.records} 条记录、${result.resources} 个资源${result.failed ? `，${result.failed} 个资源失败（详见压缩包内清单）` : ""}`;
  } catch (reason) {
    exportError.value = `完整离线归档导出失败：${String(reason)}`;
  } finally {
    exporting.value = false;
  }
}
watch(loggedIn, loadDashboard);
onMounted(loadDashboard);
onBeforeUnmount(releaseAvatars);
</script>

<template>
  <section class="hero-panel">
    <div><span class="section-kicker">{{ loggedIn ? `QQ ${user?.uin}` : "开始使用" }}</span><h2>{{ loggedIn ? `${user?.nickname}，欢迎回来` : "把珍贵的空间记忆，安全保存在本地" }}</h2><p>{{ loggedIn ? `本地已保存 ${overview.dynamics} 条动态和 ${overview.pictures} 张图片。` : "登录 QQ 空间后，可以归档动态、图片、视频和互动记录。" }}</p></div>
    <div class="hero-actions">
      <Button :label="loggedIn ? '开始归档' : '登录 QQ 空间'" :icon="loggedIn ? 'pi pi-download' : 'pi pi-link'" :loading="loading" @click="primaryAction" />
      <Button v-if="loggedIn" label="导出归档" icon="pi pi-file-export" severity="secondary" outlined :loading="exporting" :disabled="!overview.dynamics || progress.status === 'running'" @click="exportAll" />
    </div>
  </section>
  <p v-if="exportMessage" class="dashboard-export-message"><i class="pi pi-check-circle" />{{ exportMessage }}</p>
  <p v-if="exportError" class="dashboard-export-error"><i class="pi pi-exclamation-circle" />{{ exportError }}</p>
  <section class="stats-grid" aria-label="归档统计">
    <StatCard label="动态" :value="String(overview.dynamics)" :hint="overview.dynamics ? '当前账号本地归档' : '等待首次归档'" icon="pi pi-comment" tone="blue" />
    <StatCard label="照片" :value="String(overview.pictures)" :hint="overview.pictures ? '动态图片总数' : '暂无归档图片'" icon="pi pi-images" tone="purple" />
    <StatCard label="评论" :value="String(overview.comments)" :hint="`${overview.likes} 个赞`" icon="pi pi-comments" tone="green" />
    <StatCard label="本地占用" :value="storageText" hint="SQLite 资料库" icon="pi pi-database" tone="orange" />
  </section>
  <section class="dashboard-grid">
    <article class="surface-card recent-card">
      <div class="section-heading"><div><span class="section-kicker">最近动态</span><h3>归档记录</h3></div><Button label="查看全部" severity="secondary" text size="small" :disabled="!loggedIn" @click="router.push('/archives')" /></div>
      <div v-if="recent.length" class="dashboard-recent-list">
        <div v-for="item in recent" :key="item.id" class="dashboard-recent-item"><span class="dashboard-recent-avatar"><img v-if="item.authorUin && avatarSources[item.authorUin]" :src="avatarSources[item.authorUin]" /><i v-else class="pi pi-user" /></span><div><strong>{{ item.authorName || item.authorUin || "我" }}</strong><p><QzoneText :value="item.content" /></p><small>{{ formatTime(item.publishedAt) }} · {{ item.likeCount }} 赞 · {{ item.commentCount }} 评论</small></div></div>
      </div>
      <div v-else class="empty-state compact"><span><i class="pi pi-inbox" /></span><h4>{{ loggedIn ? "还没有归档记录" : "尚未登录" }}</h4><p>{{ loggedIn ? "前往任务页面创建第一个归档任务。" : "登录 QQ 空间后即可查看当前账号的归档概览。" }}</p></div>
    </article>
    <article class="surface-card ranking-card">
      <div class="section-heading"><div><span class="section-kicker">INTERACTION</span><h3>互动排行榜</h3></div><span v-if="ranking.length" class="ranking-total">TOP {{ ranking.length }}</span></div>
      <div v-if="ranking.length" class="interaction-ranking">
        <div v-for="(item, index) in ranking" :key="item.uin" class="ranking-item">
          <span class="ranking-position" :class="`rank-${index + 1}`">{{ index + 1 }}</span>
          <span class="ranking-avatar"><img v-if="avatarSources[item.uin]" :src="avatarSources[item.uin]" /><i v-else class="pi pi-user" /></span>
          <div class="ranking-person"><strong>{{ item.nickname || item.uin }}</strong><small>QQ {{ item.uin }}</small><span class="ranking-bar"><i :style="{ width: `${item.interactions / rankingMax * 100}%` }" /></span></div>
          <div class="ranking-count"><strong>{{ item.interactions }}</strong><small>{{ item.likes }} 赞 · {{ item.comments }} 评论</small></div>
        </div>
      </div>
      <div v-else class="ranking-empty"><i class="pi pi-users" /><span>{{ loggedIn ? "归档互动后将在这里生成排行" : "登录后查看互动排行" }}</span></div>
    </article>
    <article class="surface-card status-card">
      <div class="section-heading"><div><span class="section-kicker">任务状态</span><h3>空间动态归档</h3></div><Tag :value="taskText" :severity="taskSeverity" /></div>
      <div class="storage-row"><span>归档进度</span><strong>{{ progress.pages }} 页 / {{ progress.fetched }} 条</strong></div><ProgressBar :mode="progress.status === 'running' ? 'indeterminate' : 'determinate'" :value="progress.status === 'completed' ? 100 : 0" :show-value="false" />
      <ul class="status-list"><li><i class="pi pi-database" />已写入 {{ progress.saved }} 条接口记录</li><li><i class="pi pi-info-circle" />{{ progress.message }}</li><li><i :class="loggedIn ? 'pi pi-check-circle' : 'pi pi-lock'" />{{ loggedIn ? `当前账号：${user?.nickname}` : "登录后才能执行归档" }}</li></ul>
    </article>
  </section>
</template>
