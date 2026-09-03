<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { storeToRefs } from "pinia";
import Button from "primevue/button";
import Checkbox from "primevue/checkbox";
import Dialog from "primevue/dialog";
import InputText from "primevue/inputtext";
import Select from "primevue/select";
import { useAuthStore } from "../stores/auth";
import { useRecycleSessionStore } from "../stores/recycle";
import { checkRecyclePassword, closeRecyclePasswordWindow, createQzoneAlbum, listQzoneAlbums, listRecycleAlbums, listRecyclePhotos, loadRecyclePhotoPreview, openRecyclePasswordWindow, recoverRecycleAlbum, recoverRecyclePhotos } from "../utils/qzone";

interface Album { id: string; name: string; count: number }
interface Photo { id: string; sourceAlbumId: string; name: string; url?: string; deletedAt?: string }
const auth = useAuthStore();
const recycleSession = useRecycleSessionStore();
const { pwd2sig: token, ownerUin } = storeToRefs(recycleSession);
const verifying = ref(false);
const loading = ref(false);
const recovering = ref(false);
const error = ref("");
const albums = ref<Album[]>([]);
const photos = ref<Photo[]>([]);
const activeAlbumId = ref("");
const selectedIds = ref<string[]>([]);
const confirmVisible = ref(false);
const destinationMode = ref<"create" | "existing">("create");
const recoveryAlbumName = ref("恢复相册");
const destinationAlbums = ref<Album[]>([]);
const destinationAlbumId = ref("");
const destinationLoading = ref(false);
const destinationError = ref("");
const albumRecoverTarget = ref<Album | null>(null);
const albumRecovering = ref(false);
let verifyRun = 0;
const verified = computed(() => Boolean(token.value));
const allSelected = computed(() => photos.value.length > 0 && selectedIds.value.length === photos.value.length);
const activeAlbum = computed(() => albums.value.find((item) => item.id === activeAlbumId.value));
const delay = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const object = (value: unknown) => value && typeof value === "object" ? value as Record<string, unknown> : {};
function values(response: Record<string, unknown>, names: string[]) {
  const data = object(response.data);
  for (const name of names) { const value = data[name] ?? response[name]; if (Array.isArray(value)) return value.map(object); }
  return [];
}
function field(item: Record<string, unknown>, names: string[]) {
  for (const name of names) if (item[name] != null) return String(item[name]);
  return "";
}
function mapAlbums(response: Record<string, unknown>): Album[] {
  return values(response, ["album", "albums", "albumList"]).map((item) => ({ id: field(item, ["albumid", "albumId", "id"]), name: field(item, ["name", "albumname", "title"]) || "未命名相册", count: Number(field(item, ["num", "photoNum", "total"]) || 0) })).filter((item) => item.id);
}
function mapPhotos(response: Record<string, unknown>, albumId: string): Photo[] {
  return values(response, ["photo", "photos", "photoList", "list"]).map((item) => ({ id: field(item, ["lloc", "photoId", "photoid", "id"]), sourceAlbumId: field(item, ["albumid", "albumId"]) || albumId, name: field(item, ["name", "photoName", "title"]) || "照片", url: field(item, ["pre", "url", "origin_url", "rawUrl"]) || undefined, deletedAt: field(item, ["modifytime", "deleteTime", "deltime"]) || undefined })).filter((item) => item.id);
}
function responseAlbumId(response: Record<string, unknown>) {
  const candidates = [response, object(response.data), object(object(response.data).album), object(response.album)];
  for (const candidate of candidates) {
    const id = field(candidate, ["albumid", "albumId", "id"]);
    if (id) return id;
  }
  return "";
}
async function loadPhotos(albumId = "") {
  loading.value = true; error.value = ""; selectedIds.value = []; activeAlbumId.value = albumId;
  try {
    const mapped = mapPhotos(await listRecyclePhotos(token.value, albumId || undefined), albumId);
    photos.value = mapped;
    photos.value = await Promise.all(mapped.map(async (photo) => {
      if (!photo.url || photo.url.startsWith("data:")) return photo;
      try { return { ...photo, url: await loadRecyclePhotoPreview(photo.url) }; } catch { return photo; }
    }));
  }
  catch (reason) { error.value = String(reason); }
  finally { loading.value = false; }
}
async function loadAll() {
  loading.value = true; error.value = "";
  try { albums.value = mapAlbums(await listRecycleAlbums(token.value)); await loadPhotos(); }
  catch (reason) { error.value = String(reason); loading.value = false; }
}
async function cancelVerify() {
  verifyRun += 1;
  verifying.value = false;
  try { await closeRecyclePasswordWindow(); }
  catch (reason) { error.value = String(reason); }
}
async function verify() {
  if (!auth.loggedIn) { await auth.openLogin(); return; }
  const run = ++verifyRun; verifying.value = true; error.value = "";
  try {
    await openRecyclePasswordWindow();
    while (run === verifyRun && verifying.value) {
      const result = await checkRecyclePassword();
      if (result.token) { recycleSession.setVerified(result.token, auth.user?.uin ?? ""); verifying.value = false; await closeRecyclePasswordWindow(); await loadAll(); return; }
      if (!result.windowOpen) { error.value = "验证窗口已关闭，请重新点击验证"; verifying.value = false; return; }
      await delay(1200);
    }
  } catch (reason) { error.value = String(reason); verifying.value = false; }
}
function toggleAll() { selectedIds.value = allSelected.value ? [] : photos.value.map((item) => item.id); }
async function openRecoverDialog() {
  destinationMode.value = "existing";
  recoveryAlbumName.value = "恢复相册";
  destinationError.value = "";
  confirmVisible.value = true;
  await selectDestinationMode("existing");
}
async function selectDestinationMode(mode: "create" | "existing") {
  destinationMode.value = mode;
  if (mode !== "existing" || destinationLoading.value || destinationAlbums.value.length) return;
  destinationError.value = "";
  destinationLoading.value = true;
  try {
    destinationAlbums.value = mapAlbums(await listQzoneAlbums());
    destinationAlbumId.value = destinationAlbums.value[0]?.id ?? "";
  } catch (reason) {
    destinationAlbums.value = [];
    destinationAlbumId.value = "";
    destinationError.value = `已有相册加载失败：${String(reason)}`;
  } finally {
    destinationLoading.value = false;
  }
}
async function recover() {
  const groups = new Map<string, string[]>();
  for (const photo of photos.value.filter((item) => selectedIds.value.includes(item.id))) {
    const group = groups.get(photo.sourceAlbumId) ?? [];
    group.push(photo.id); groups.set(photo.sourceAlbumId, group);
  }
  recovering.value = true; error.value = "";
  try {
    let targetAlbumId = destinationAlbumId.value;
    if (destinationMode.value === "create") {
      const name = recoveryAlbumName.value.trim();
      if (!name) throw new Error("请输入新相册名称");
      const beforeAlbums = mapAlbums(await listQzoneAlbums());
      const previousIds = new Set(beforeAlbums.map((album) => album.id));
      const created = await createQzoneAlbum(name);
      targetAlbumId = "";
      for (let attempt = 0; attempt < 3 && !targetAlbumId; attempt += 1) {
        const refreshed = mapAlbums(await listQzoneAlbums());
        targetAlbumId = refreshed.find((album) => album.name === name && !previousIds.has(album.id))?.id ?? "";
        if (!targetAlbumId && attempt < 2) await delay(300 * (attempt + 1));
      }
      targetAlbumId ||= responseAlbumId(created);
      if (!targetAlbumId) throw new Error("相册已创建，但响应中缺少相册 ID，请刷新后重试");
      // 新相册会先出现在列表服务中，照片恢复服务通常还需要短暂同步。
      await delay(1800);
    } else if (!targetAlbumId) {
      throw new Error("请选择恢复目标相册");
    }
    for (const [sourceAlbumId, ids] of groups) {
      if (!sourceAlbumId) throw new Error("照片缺少回收站来源相册 ID");
      await recoverRecyclePhotos(token.value, sourceAlbumId, targetAlbumId, ids);
    }
    confirmVisible.value = false; await loadPhotos(activeAlbumId.value);
  }
  catch (reason) { error.value = String(reason); }
  finally { recovering.value = false; }
}
function openAlbumRecoverDialog(album: Album) { albumRecoverTarget.value = album; }
async function recoverAlbum() {
  if (!albumRecoverTarget.value) return;
  albumRecovering.value = true; error.value = "";
  try {
    await recoverRecycleAlbum(token.value, albumRecoverTarget.value.id);
    albumRecoverTarget.value = null;
    await loadAll();
  } catch (reason) { error.value = String(reason); }
  finally { albumRecovering.value = false; }
}
function resetVerification() { recycleSession.clear(); photos.value = []; albums.value = []; selectedIds.value = []; }
onMounted(() => {
  if (token.value && ownerUin.value === (auth.user?.uin ?? "")) void loadAll();
  else if (token.value) recycleSession.clear();
});
onBeforeUnmount(() => { verifyRun += 1; verifying.value = false; void closeRecyclePasswordWindow(); });
</script>

<template>
  <div class="recycle-page">
    <section v-if="!verified" class="surface-card empty-state recycle-auth-state">
      <span><i class="pi pi-lock" /></span><h2>需要验证 QQ 空间独立密码</h2>
      <p>验证将在独立的内置浏览器窗口中完成。应用不会读取或保存你的密码，只接收返回的临时验证签名。</p>
      <div class="recycle-auth-actions">
        <Button :label="verifying ? '等待验证完成…' : (auth.loggedIn ? '验证独立密码' : '先登录 QQ 空间')" icon="pi pi-shield" :loading="verifying" :disabled="verifying" @click="verify" />
        <Button v-if="verifying" label="取消" severity="secondary" text @click="cancelVerify" />
      </div>
      <small v-if="verifying" class="recycle-auth-tip">请在弹出的验证窗口中输入独立密码。若窗口未正常显示，请取消后重试。</small><p v-if="error" class="recycle-error">{{ error }}</p>
    </section>
    <template v-else>
      <section class="surface-card recycle-toolbar">
        <div><h2>相册回收站</h2><p>{{ photos.length }} 张可恢复照片<span v-if="activeAlbum"> · {{ activeAlbum.name }}</span></p></div>
        <div class="recycle-toolbar-actions"><Button label="重新验证" icon="pi pi-key" severity="secondary" text @click="resetVerification" /><Button label="刷新" icon="pi pi-refresh" severity="secondary" text :loading="loading" @click="loadAll" /><Button v-if="activeAlbum" label="恢复整个相册" icon="pi pi-folder-open" :loading="albumRecovering" :disabled="albumRecovering" @click="openAlbumRecoverDialog(activeAlbum)" /><Button v-else :label="`恢复所选 (${selectedIds.length})`" icon="pi pi-replay" :disabled="!selectedIds.length" @click="openRecoverDialog" /></div>
      </section>
      <p v-if="error" class="surface-card recycle-error">{{ error }}</p>
      <div class="recycle-layout">
        <aside class="surface-card recycle-albums">
          <button :class="{ active: !activeAlbumId }" type="button" @click="loadPhotos()"><i class="pi pi-images" /><span><strong>全部照片</strong><small>所有相册</small></span></button>
          <button v-for="album in albums" :key="album.id" :class="{ active: activeAlbumId === album.id }" type="button" @click="loadPhotos(album.id)"><i class="pi pi-folder" /><span><strong>{{ album.name }}</strong><small>{{ album.count ? `${album.count} 张` : '回收站相册' }}</small></span></button>
        </aside>
        <section class="surface-card recycle-content">
          <div v-if="activeAlbum" class="recycle-mode-notice"><i class="pi pi-info-circle" /><div><strong>当前为整册恢复模式</strong><span>QQ 空间只支持将此相册整体恢复。若要把单张照片恢复到指定相册，请切换到“全部照片”。</span></div><Button label="查看全部照片" severity="secondary" size="small" @click="loadPhotos()" /></div>
          <div v-if="photos.length && !activeAlbum" class="recycle-select-row"><Checkbox :model-value="allSelected" binary input-id="select-all-recycle" @update:model-value="toggleAll" /><label for="select-all-recycle">全选当前照片</label></div>
          <div v-if="loading" class="empty-state compact"><span><i class="pi pi-spin pi-spinner" /></span><h4>正在读取回收站</h4></div>
          <div v-else-if="!photos.length" class="empty-state compact"><span><i class="pi pi-check-circle" /></span><h4>回收站中没有照片</h4><p>当前筛选下没有可恢复的照片。</p></div>
          <div v-else class="recycle-photo-grid"><label v-for="photo in photos" :key="photo.id" class="recycle-photo-card" :class="{ selected: selectedIds.includes(photo.id), selectable: !activeAlbum }"><img v-if="photo.url" :src="photo.url" :alt="photo.name" loading="lazy" /><span v-else class="recycle-photo-placeholder"><i class="pi pi-image" /></span><span v-if="!activeAlbum" class="recycle-photo-check"><Checkbox v-model="selectedIds" :value="photo.id" /></span><span class="recycle-photo-copy"><strong>{{ photo.name }}</strong><small v-if="photo.deletedAt">{{ photo.deletedAt }}</small></span></label></div>
        </section>
      </div>
    </template>
    <Dialog v-model:visible="confirmVisible" modal :closable="!recovering" :draggable="false" class="delete-dialog recycle-recover-dialog" header="选择恢复目标">
      <div class="recycle-destination-tabs"><button type="button" :class="{ active: destinationMode === 'create' }" @click="selectDestinationMode('create')">新建相册</button><button type="button" :class="{ active: destinationMode === 'existing' }" @click="selectDestinationMode('existing')">已有相册</button></div>
      <div v-if="destinationMode === 'create'" class="recycle-destination-field"><label for="recovery-album-name">相册名称</label><InputText id="recovery-album-name" v-model="recoveryAlbumName" maxlength="30" fluid /><small>将新建一个所有人可见的公开相册。</small></div>
      <div v-else class="recycle-destination-field"><label for="recovery-album-select">目标相册</label><Select id="recovery-album-select" v-model="destinationAlbumId" :options="destinationAlbums" option-label="name" option-value="id" :loading="destinationLoading" :disabled="destinationLoading || !destinationAlbums.length" placeholder="请选择相册" fluid /><small v-if="!destinationLoading && !destinationAlbums.length">暂无可选相册，你可以返回新建相册。</small></div>
      <p v-if="destinationError && destinationMode === 'existing'" class="recycle-dialog-error">{{ destinationError }}</p><p class="recycle-dialog-summary">将恢复 {{ selectedIds.length }} 张照片，成功后它们会从回收站中移除。</p>
      <template #footer><Button label="取消" severity="secondary" text :disabled="recovering" @click="confirmVisible = false" /><Button label="确认恢复" icon="pi pi-replay" :loading="recovering" :disabled="destinationMode === 'existing' && !destinationAlbumId" @click="recover" /></template>
    </Dialog>
    <Dialog :visible="Boolean(albumRecoverTarget)" modal :closable="!albumRecovering" :draggable="false" class="delete-dialog" header="恢复整个相册？" @update:visible="(visible) => { if (!visible && !albumRecovering) albumRecoverTarget = null; }">
      <p>将恢复回收站相册“{{ albumRecoverTarget?.name }}”中的全部内容。恢复后该相册会从回收站移除。</p>
      <template #footer><Button label="取消" severity="secondary" text :disabled="albumRecovering" @click="albumRecoverTarget = null" /><Button label="确认恢复相册" icon="pi pi-folder-open" :loading="albumRecovering" @click="recoverAlbum" /></template>
    </Dialog>
  </div>
</template>
