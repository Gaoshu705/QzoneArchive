<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { platform } from "@tauri-apps/plugin-os";
import { convertFileSrc } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { writeFile } from "@tauri-apps/plugin-fs";
import Button from "primevue/button";
import Checkbox from "primevue/checkbox";
import Dialog from "primevue/dialog";
import InputText from "primevue/inputtext";
import Paginator, { type PageState } from "primevue/paginator";
import QzoneText from "../components/QzoneText.vue";
import { loadRemoteImageBlob } from "../utils/archiveImage";
import { useAuthStore } from "../stores/auth";
import { clearArchivedFeeds, countArchivedFeeds, deleteArchivedFeeds, exportArchivedZip, listArchivedFeeds, loadArchivedImage, loadArchivedVideo, type ArchiveCategory, type ArchiveItem } from "../utils/qzone";

type DeleteAction = "selected" | "all";
const authStore = useAuthStore();
const records = ref<ArchiveItem[]>([]);
const query = ref("");
const loading = ref(false);
const deleting = ref(false);
const exporting = ref(false);
const error = ref("");
const exportNotice = ref("");
const selectedIds = ref<number[]>([]);
const confirmVisible = ref(false);
const pendingAction = ref<DeleteAction>("selected");
const first = ref(0);
const pageSize = ref(20);
const totalRecords = ref(0);
const category = ref<ArchiveCategory>("self");
const categoryOptions: { label: string; value: ArchiveCategory; icon: string; hint: string }[] = [
  { label: "本人动态", value: "self", icon: "pi pi-user", hint: "由当前账号发布" },
  { label: "其他动态", value: "other", icon: "pi pi-users", hint: "好友及其他用户" },
  { label: "留言", value: "guestbook", icon: "pi pi-envelope", hint: "空间留言板内容" },
];
const categoryLabel = computed(() => categoryOptions.find((item) => item.value === category.value)?.label || "归档");
const avatarTimestamp = ref(Date.now());
const videoSources = reactive<Record<number, string>>({});
const videoLoading = reactive<Record<number, boolean>>({});
const videoErrors = reactive<Record<number, string>>({});
const imageSources = reactive<Record<string, string>>({});
const imageLoading = reactive<Record<string, boolean>>({});
const imageErrors = reactive<Record<string, string>>({});
const imageFallbackAttempted = reactive<Record<string, boolean>>({});
const imagePreviewVisible = ref(false);
const previewImageUrl = ref("");
const previewImageName = ref("qzone-image.jpg");
const savingImage = ref(false);
const imageScale = ref(1);
const imageOffset = reactive({ x: 0, y: 0 });
const imageActionVisible = ref(false);
const imagePointers = new Map<number, { x: number; y: number }>();
let imageDragStart: { x: number; y: number; offsetX: number; offsetY: number } | undefined;
let pinchStart: { distance: number; scale: number } | undefined;
let longPressTimer: ReturnType<typeof setTimeout> | undefined;
const expandedComments = reactive(new Set<number>());
const expandedLikes = reactive(new Set<number>());
let imageObserver: IntersectionObserver | undefined;
const currentPlatform = platform();
const desktopPlatforms = new Set(["windows", "macos", "linux"]);
const isDesktopPlatform = desktopPlatforms.has(currentPlatform);

const filtered = computed(() => {
  const key = query.value.trim().toLowerCase();
  return key ? records.value.filter((item) => [item.content, item.authorName, item.authorUin].some((value) => value?.toLowerCase().includes(key))) : records.value;
});
const visibleIds = computed(() => filtered.value.map((item) => item.id));
const allVisibleSelected = computed(() => visibleIds.value.length > 0 && visibleIds.value.every((id) => selectedIds.value.includes(id)));
const confirmTitle = computed(() => pendingAction.value === "all" ? "清空全部归档？" : `删除 ${selectedIds.value.length} 条归档？`);
const confirmDescription = computed(() => pendingAction.value === "all" ? "将永久删除当前账号的全部归档记录。" : "选中的归档记录将被永久删除。");
const formatTime = (seconds: number) => seconds ? new Intl.DateTimeFormat("zh-CN", { dateStyle: "medium", timeStyle: "short" }).format(new Date(seconds * 1000)) : "时间未知";
const avatarUrl = (uin?: string) => uin ? `https://qlogo2.store.qq.com/qzone/${uin}/${uin}/50?${avatarTimestamp.value}` : "";

async function load() {
  releaseVideos();
  loading.value = true; error.value = "";
  try {
    const [items, total] = await Promise.all([listArchivedFeeds(pageSize.value, first.value, category.value), countArchivedFeeds(category.value)]);
    records.value = items; totalRecords.value = total; selectedIds.value = []; avatarTimestamp.value = Date.now();
    if (!items.length && first.value > 0 && total > 0) { first.value = Math.max(0, Math.floor((total - 1) / pageSize.value) * pageSize.value); await load(); }
    else { await nextTick(); observeArchiveImages(); }
  }
  catch (reason) { error.value = String(reason); }
  finally { loading.value = false; }
}
function releaseVideos() {
  imageObserver?.disconnect();
  Object.values(videoSources).forEach((url) => { if (url.startsWith("blob:")) URL.revokeObjectURL(url); });
  Object.keys(videoSources).forEach((key) => delete videoSources[Number(key)]);
  Object.keys(videoErrors).forEach((key) => delete videoErrors[Number(key)]);
  Object.values(imageSources).forEach((url) => { if (url.startsWith("blob:")) URL.revokeObjectURL(url); });
  Object.keys(imageSources).forEach((key) => delete imageSources[key]);
  Object.keys(imageLoading).forEach((key) => delete imageLoading[key]);
  Object.keys(imageErrors).forEach((key) => delete imageErrors[key]);
  Object.keys(imageFallbackAttempted).forEach((key) => delete imageFallbackAttempted[key]);
}
function observeArchiveImages() {
  imageObserver?.disconnect();
  imageObserver = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      const element = entry.target as HTMLElement;
      const url = element.dataset.archiveImage;
      const dynamicId = Number(element.dataset.dynamicId);
      const pictureIndex = Number(element.dataset.pictureIndex);
      if (url) void loadArchiveImage(url, Number.isFinite(dynamicId) ? dynamicId : undefined, Number.isFinite(pictureIndex) ? pictureIndex : undefined);
      imageObserver?.unobserve(element);
    }
  }, { rootMargin: "240px 0px" });
  document.querySelectorAll<HTMLElement>(".archive-page [data-archive-image]").forEach((element) => imageObserver?.observe(element));
}
async function loadArchiveImage(url: string, dynamicId?: number, pictureIndex?: number) {
  if (!url || imageSources[url] || imageLoading[url]) return;
  imageLoading[url] = true;
  delete imageErrors[url];
  delete imageFallbackAttempted[url];
  try {
    if (dynamicId !== undefined && pictureIndex !== undefined) {
      try {
        imageSources[url] = convertFileSrc(await loadArchivedImage(dynamicId, pictureIndex));
        return;
      } catch (reason) {
        console.warn("本地图片归档加载失败，改用原始地址", reason);
      }
    }
    await loadRemoteArchiveImage(url);
  } catch (reason) { imageErrors[url] = String(reason); console.error("归档图片加载失败", reason); }
  finally { imageLoading[url] = false; }
}
async function loadRemoteArchiveImage(url: string) {
  imageSources[url] = await loadRemoteImageBlob(url);
}
async function handleArchiveImageError(url: string) {
  if (!url || imageLoading[url]) return;
  const source = imageSources[url];
  if (source?.startsWith("blob:")) URL.revokeObjectURL(source);
  delete imageSources[url];
  if (imageFallbackAttempted[url]) {
    imageErrors[url] = "图片文件无法显示，可点击重试";
    return;
  }
  imageFallbackAttempted[url] = true;
  imageLoading[url] = true;
  try { await loadRemoteArchiveImage(url); }
  catch (reason) { imageErrors[url] = String(reason); }
  finally { imageLoading[url] = false; }
}
function imageExtension(url: string) {
  const match = url.match(/\.([a-zA-Z0-9]{2,5})(?:[?#]|$)/);
  const extension = match?.[1]?.toLowerCase();
  return extension && ["jpg", "jpeg", "png", "webp", "gif", "avif"].includes(extension) ? extension : "jpg";
}
function openImagePreview(sourceUrl: string, originalUrl: string, cellId: string, index: number) {
  previewImageUrl.value = sourceUrl;
  previewImageName.value = `qzone-${cellId.replace(/[^a-zA-Z0-9_-]/g, "_")}-${index + 1}.${imageExtension(originalUrl)}`;
  resetImageTransform();
  imagePreviewVisible.value = true;
}
function resetImageTransform() { imageScale.value = 1; imageOffset.x = 0; imageOffset.y = 0; }
function setImageScale(value: number) {
  imageScale.value = Math.min(5, Math.max(0.5, value));
  if (imageScale.value <= 1) { imageOffset.x = 0; imageOffset.y = 0; }
}
function closeImagePreview() { imagePreviewVisible.value = false; imageActionVisible.value = false; resetImageTransform(); }
function imageWheel(event: WheelEvent) { setImageScale(imageScale.value * (event.deltaY < 0 ? 1.15 : 0.87)); }
function toggleImageZoom() { imageScale.value > 1 ? resetImageTransform() : setImageScale(2); }
function pointerDistance() {
  const points = [...imagePointers.values()];
  return points.length < 2 ? 0 : Math.hypot(points[0].x - points[1].x, points[0].y - points[1].y);
}
function clearLongPress() { if (longPressTimer) clearTimeout(longPressTimer); longPressTimer = undefined; }
function imagePointerDown(event: PointerEvent) {
  (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  imagePointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
  imageDragStart = { x: event.clientX, y: event.clientY, offsetX: imageOffset.x, offsetY: imageOffset.y };
  if (imagePointers.size === 2) { pinchStart = { distance: pointerDistance(), scale: imageScale.value }; clearLongPress(); }
  else if (event.pointerType !== "mouse") {
    clearLongPress();
    longPressTimer = setTimeout(() => { imageActionVisible.value = true; navigator.vibrate?.(25); }, 550);
  }
}
function imagePointerMove(event: PointerEvent) {
  const previous = imagePointers.get(event.pointerId);
  if (!previous) return;
  if (Math.hypot(event.clientX - previous.x, event.clientY - previous.y) > 7) clearLongPress();
  imagePointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
  if (imagePointers.size >= 2 && pinchStart?.distance) setImageScale(pinchStart.scale * pointerDistance() / pinchStart.distance);
  else if (imageScale.value > 1 && imageDragStart) {
    imageOffset.x = imageDragStart.offsetX + event.clientX - imageDragStart.x;
    imageOffset.y = imageDragStart.offsetY + event.clientY - imageDragStart.y;
  }
}
function imagePointerUp(event: PointerEvent) {
  clearLongPress(); imagePointers.delete(event.pointerId);
  if (imagePointers.size < 2) pinchStart = undefined;
  if (!imagePointers.size) imageDragStart = undefined;
}
async function savePreviewImage() {
  if (!previewImageUrl.value || savingImage.value) return;
  savingImage.value = true; error.value = "";
  try {
    const extension = previewImageName.value.split(".").pop() || "jpg";
    const path = await save({ defaultPath: previewImageName.value, filters: [{ name: "图片", extensions: [extension, "jpg", "png", "webp"] }] });
    if (!path) return;
    const response = await window.fetch(previewImageUrl.value);
    await writeFile(path, new Uint8Array(await response.arrayBuffer()));
  } catch (reason) { error.value = `保存图片失败：${String(reason)}`; }
  finally { savingImage.value = false; }
}
async function exportZip(selectedOnly: boolean) {
  if (exporting.value || (selectedOnly && !selectedIds.value.length)) return;
  exporting.value = true; error.value = ""; exportNotice.value = "";
  try {
    const date = new Date().toISOString().slice(0, 10);
    const path = await save({
      defaultPath: `QQ空间归档-${categoryLabel.value}-${date}.zip`,
      filters: [{ name: "ZIP 离线归档", extensions: ["zip"] }],
    });
    if (!path) return;
    const result = await exportArchivedZip(category.value, path, selectedOnly ? selectedIds.value : undefined);
    exportNotice.value = `离线 ZIP 导出完成：${result.records} 条归档、${result.resources} 个资源${result.failed ? `，${result.failed} 个资源失败（详见压缩包内清单）` : ""}`;
  } catch (reason) { error.value = `导出失败：${String(reason)}`; }
  finally { exporting.value = false; }
}
async function loadVideo(item: ArchiveItem) {
  if (!item.videoUrl || videoLoading[item.id]) return;
  videoLoading[item.id] = true; delete videoErrors[item.id];
  try {
    const path = await loadArchivedVideo(item.id);
    videoSources[item.id] = convertFileSrc(path);
    await nextTick();
    const video = document.querySelector<HTMLVideoElement>(`#archive-video-${item.id}`);
    if (!video) throw new Error("视频播放器创建失败");
    await video.play().catch((reason) => { throw new Error(`播放器无法播放该视频：${String(reason)}`); });
  } catch (reason) { videoErrors[item.id] = `视频加载失败：${String(reason)}`; }
  finally { videoLoading[item.id] = false; }
}
function toggleVisible() {
  const visible = new Set(visibleIds.value);
  selectedIds.value = allVisibleSelected.value ? selectedIds.value.filter((id) => !visible.has(id)) : [...new Set([...selectedIds.value, ...visible])];
}
function askDelete(action: DeleteAction) { pendingAction.value = action; confirmVisible.value = true; }
function toggleComments(id: number) { expandedComments.has(id) ? expandedComments.delete(id) : expandedComments.add(id); }
function changePage(event: PageState) { first.value = event.first; pageSize.value = event.rows; void load(); document.querySelector('.app-shell')?.scrollTo(0, 0); }
async function confirmDelete() {
  deleting.value = true; error.value = "";
  try {
    if (pendingAction.value === "all") await clearArchivedFeeds();
    else await deleteArchivedFeeds(selectedIds.value);
    confirmVisible.value = false;
    await load();
  } catch (reason) { error.value = String(reason); }
  finally { deleting.value = false; }
}
onMounted(async () => { if (authStore.loggedIn) await load(); });
watch(category, () => { first.value = 0; query.value = ""; void load(); });
onBeforeUnmount(() => { clearLongPress(); imageObserver?.disconnect(); releaseVideos(); });
</script>

<template>
  <div class="archive-page" :class="isDesktopPlatform ? 'platform-desktop' : 'platform-mobile'">
  <section class="archive-header surface-card">
    <div class="archive-header-copy">
      <span class="archive-header-icon"><i class="pi pi-box" /></span>
      <div><p class="section-kicker">LOCAL ARCHIVE</p><h2>归档内容</h2><p>{{ categoryLabel }}共 {{ totalRecords }} 条，可搜索或批量管理。</p></div>
    </div>
    <div class="archive-header-actions">
      <Button icon="pi pi-refresh" label="刷新" severity="secondary" text :loading="loading" @click="load" />
      <Button icon="pi pi-file-export" label="导出全部 ZIP" severity="secondary" text :loading="exporting" :disabled="!totalRecords || loading" @click="exportZip(false)" />
      <Button icon="pi pi-trash" label="清空归档" severity="danger" text :disabled="!totalRecords || loading" @click="askDelete('all')" />
    </div>
  </section>

  <section class="archive-toolbar surface-card">
    <nav class="archive-category-tabs" aria-label="归档分类">
      <button v-for="option in categoryOptions" :key="option.value" type="button" class="archive-category-tab" :class="{ 'is-active': category === option.value }" :aria-current="category === option.value ? 'page' : undefined" @click="category = option.value">
        <span class="archive-category-icon"><i :class="option.icon" /></span>
        <span><strong>{{ option.label }}</strong><small>{{ option.hint }}</small></span>
        <i v-if="category === option.value" class="pi pi-check archive-category-check" />
      </button>
    </nav>
    <div class="archive-toolbar-divider" />
    <div class="archive-control-bar">
      <div class="search-box"><i class="pi pi-search" /><InputText v-model="query" placeholder="搜索内容、用户或 QQ 号" /></div>
      <div v-if="records.length" class="selection-controls">
        <Button :label="allVisibleSelected ? '取消全选' : '全选'" icon="pi pi-check-square" severity="secondary" outlined size="small" @click="toggleVisible" />
        <span v-if="selectedIds.length" class="selection-count">已选 {{ selectedIds.length }} 条</span>
        <Button v-if="selectedIds.length" label="导出选中 ZIP" icon="pi pi-file-export" severity="secondary" size="small" :loading="exporting" @click="exportZip(true)" />
        <Button v-if="selectedIds.length" label="删除所选" icon="pi pi-trash" severity="danger" size="small" @click="askDelete('selected')" />
      </div>
    </div>
  </section>

  <p v-if="error" class="archive-error"><i class="pi pi-exclamation-circle" />{{ error }}</p>
  <p v-if="exportNotice" class="archive-export-notice"><i class="pi pi-check-circle" />{{ exportNotice }}</p>
  <section v-if="filtered.length" class="archive-list">
    <article v-for="item in filtered" :key="item.id" class="surface-card archive-card" :class="{ 'archive-card-selected': selectedIds.includes(item.id) }">
      <Checkbox v-model="selectedIds" class="archive-checkbox" :input-id="`archive-${item.id}`" :value="item.id" />
      <div class="archive-card-body">
        <header class="archive-dynamic-header"><span class="archive-avatar archive-publisher-avatar"><img v-if="item.authorUin" :src="avatarUrl(item.authorUin)" loading="lazy" referrerpolicy="no-referrer" /><i v-else class="pi pi-user" /></span><div class="archive-publisher"><strong>{{ item.authorName || "我" }}</strong><span><span v-if="item.authorUin">QQ {{ item.authorUin }}</span><span>{{ formatTime(item.publishedAt) }}</span></span></div></header>
        <p class="archive-dynamic-content"><QzoneText :value="item.content" /></p>
        <div v-if="item.pictureUrls.length" class="archive-picture-grid" :class="`pictures-${Math.min(item.pictureUrls.length, 4)}`"><template v-for="(url, pictureIndex) in item.pictureUrls.slice(0, 4)" :key="url"><button v-if="imageSources[url]" type="button" class="archive-picture-button" :aria-label="`查看动态图片 ${pictureIndex + 1}`" @click="openImagePreview(imageSources[url], url, item.cellId, pictureIndex)"><img :src="imageSources[url]" :alt="`动态图片 ${pictureIndex + 1}`" @error="handleArchiveImageError(url)" /></button><div v-else class="archive-image-loading" :data-archive-image="url" :data-dynamic-id="item.id" :data-picture-index="pictureIndex"><i class="pi pi-image" /><button v-if="imageErrors[url]" type="button" class="archive-image-retry" :title="imageErrors[url]" @click.stop="loadArchiveImage(url, item.id, pictureIndex)">加载失败，重试</button><span v-else>{{ imageLoading[url] ? "正在尝试多个图片地址" : "等待加载" }}</span></div></template><span v-if="item.pictureUrls.length > 4" class="picture-more">+{{ item.pictureUrls.length - 4 }}</span></div>
        <video v-if="videoSources[item.id]" :id="`archive-video-${item.id}`" class="archive-video" :src="videoSources[item.id]" controls preload="metadata" playsinline />
        <div v-else-if="item.videoUrl" class="archive-video-cover" :class="{ 'is-loading': videoLoading[item.id] }" :data-archive-image="item.videoCoverUrl || undefined" role="button" tabindex="0" @click="loadVideo(item)" @keydown.enter="loadVideo(item)">
          <img v-if="item.videoCoverUrl && imageSources[item.videoCoverUrl]" :src="imageSources[item.videoCoverUrl]" @error="handleArchiveImageError(item.videoCoverUrl)" />
          <div class="video-cover-shade"><span class="video-play-button"><i :class="videoLoading[item.id] ? 'pi pi-spin pi-spinner' : 'pi pi-play'" /></span><strong>{{ videoLoading[item.id] ? "正在加载视频…" : "点击播放" }}</strong><small>{{ videoErrors[item.id] || "视频将在点击后下载" }}</small></div>
        </div>
        <div class="archive-assets">
          <span class="archive-likes" v-if="item.likes.length">
            <i class="pi pi-heart" />
            <template v-if="item.likes.length <= 10 || expandedLikes.has(item.id)">
              <span class="archive-like-names">{{ item.likes.map(l => l.nickname || l.uin || 'QQ用户').join('、') }}</span>
              <span> 赞了</span>
              <button v-if="item.likes.length > 10" type="button" class="archive-like-toggle" @click="expandedLikes.delete(item.id)">收起</button>
            </template>
            <template v-else>
              <span class="archive-like-names">{{ item.likes.slice(0, 10).map(l => l.nickname || l.uin || 'QQ用户').join('、') }}</span>
              <button type="button" class="archive-like-toggle" @click="expandedLikes.add(item.id)">等 {{ item.likeCount }} 人赞了</button>
            </template>
          </span>
          <span v-if="item.commentCount"><i class="pi pi-comment" />{{ item.commentCount }} 条评论</span>
          <span v-if="item.pictureUrls.length"><i class="pi pi-images" />{{ item.pictureUrls.length }} 张图片</span>
          <span v-if="item.videoUrl"><i class="pi pi-video" />视频</span></div>
        <section v-if="item.comments.length" class="archive-comments">
          <div v-for="comment in (expandedComments.has(item.id) ? item.comments : item.comments.slice(0, 3))" :key="`${comment.uin}-${comment.createdAt}-${comment.content}`" class="archive-comment">
            <span class="comment-avatar"><img v-if="comment.uin" :src="avatarUrl(comment.uin)" loading="lazy" referrerpolicy="no-referrer" /><i v-else class="pi pi-user" /></span>
            <div class="archive-comment-body"><div class="archive-comment-meta"><strong>{{ comment.nickname || comment.uin || "QQ 用户" }}</strong><span>评论于</span><time>{{ formatTime(comment.createdAt) }}</time></div><p><QzoneText :value="comment.content" /></p>
              <div v-if="comment.replies.length" class="archive-comment-replies">
                <div v-for="(reply, replyIndex) in comment.replies" :key="`${reply.uin}-${reply.createdAt}-${replyIndex}`" class="archive-reply">
                  <span class="reply-avatar"><img v-if="reply.uin" :src="avatarUrl(reply.uin)" loading="lazy" referrerpolicy="no-referrer" /><i v-else class="pi pi-user" /></span>
                  <div><div class="archive-comment-meta"><strong>{{ reply.nickname || reply.uin || "QQ 用户" }}</strong><span>回复 {{ reply.replyToNickname || reply.replyToUin || comment.nickname || comment.uin || "QQ 用户" }}</span><time>{{ formatTime(reply.createdAt) }}</time></div><p><QzoneText :value="reply.content" /></p></div>
                </div>
              </div>
            </div>
          </div>
          <Button v-if="item.comments.length > 3" :label="expandedComments.has(item.id) ? '收起评论' : `查看全部 ${item.comments.length} 条评论`" :icon="expandedComments.has(item.id) ? 'pi pi-angle-up' : 'pi pi-angle-down'" severity="secondary" text size="small" @click="toggleComments(item.id)" />
        </section>
      </div>
    </article>
  </section>
  <section v-else class="surface-card empty-state page-empty"><span><i class="pi pi-folder-open" /></span><h2>{{ query ? "没有匹配的记录" : `暂无${categoryLabel}` }}</h2><p>{{ query ? "尝试更换搜索关键词。" : "请先前往任务页执行归档。" }}</p></section>
  <Paginator v-if="totalRecords > pageSize" class="archive-paginator" :first="first" :rows="pageSize" :total-records="totalRecords" :rows-per-page-options="[10, 20, 30, 50]" template="FirstPageLink PrevPageLink PageLinks NextPageLink LastPageLink RowsPerPageDropdown CurrentPageReport" current-page-report-template="{first} - {last} / 共 {totalRecords} 条" @page="changePage" />

  <Dialog v-model:visible="confirmVisible" modal :closable="!deleting" :draggable="false" class="delete-dialog" :header="confirmTitle">
    <div class="delete-dialog-content"><span class="delete-warning"><i class="pi pi-trash" /></span><div><p>{{ confirmDescription }}</p><small>此操作无法撤销，请确认后继续。</small></div></div>
    <template #footer><Button label="取消" severity="secondary" text :disabled="deleting" @click="confirmVisible = false" /><Button label="确认删除" icon="pi pi-trash" severity="danger" :loading="deleting" @click="confirmDelete" /></template>
  </Dialog>
  <Teleport to="body">
    <Transition name="image-viewer">
      <div v-if="imagePreviewVisible" class="wechat-image-viewer" role="dialog" aria-modal="true" aria-label="查看图片" @click.self="closeImagePreview">
        <button class="image-viewer-close" type="button" aria-label="关闭" @click="closeImagePreview"><i class="pi pi-times" /></button>
        <div class="image-viewer-stage" @wheel.prevent="imageWheel" @click.self="closeImagePreview">
          <img v-if="previewImageUrl" class="image-viewer-picture" :class="{ 'is-zoomed': imageScale > 1 }" :src="previewImageUrl" alt="归档图片大图" draggable="false" :style="{ transform: `translate3d(${imageOffset.x}px, ${imageOffset.y}px, 0) scale(${imageScale})` }" @dblclick.prevent="toggleImageZoom" @pointerdown.prevent="imagePointerDown" @pointermove.prevent="imagePointerMove" @pointerup="imagePointerUp" @pointercancel="imagePointerUp" />
        </div>
        <div class="image-viewer-tools">
          <button type="button" aria-label="缩小" @click="setImageScale(imageScale / 1.25)"><i class="pi pi-minus" /></button>
          <span>{{ Math.round(imageScale * 100) }}%</span>
          <button type="button" aria-label="放大" @click="setImageScale(imageScale * 1.25)"><i class="pi pi-plus" /></button>
          <button type="button" aria-label="恢复原始大小" @click="resetImageTransform"><i class="pi pi-refresh" /></button>
          <button type="button" aria-label="保存图片" :disabled="savingImage" @click="savePreviewImage"><i :class="savingImage ? 'pi pi-spin pi-spinner' : 'pi pi-download'" /></button>
        </div>
        <p class="image-viewer-tip">双击或双指缩放 · 长按保存</p>
        <Transition name="image-sheet">
          <div v-if="imageActionVisible" class="image-action-mask" @click.self="imageActionVisible = false">
            <div class="image-action-sheet">
              <button type="button" :disabled="savingImage" @click="savePreviewImage"><i class="pi pi-download" /><span>{{ savingImage ? "正在保存…" : "保存图片" }}</span></button>
              <button type="button" @click="imageActionVisible = false">取消</button>
            </div>
          </div>
        </Transition>
      </div>
    </Transition>
  </Teleport>
  </div>
</template>
