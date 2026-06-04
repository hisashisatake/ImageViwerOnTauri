<script lang="ts">
  import { Channel, convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onDestroy, onMount } from "svelte";
  import PageView from "./page.view.svelte";
  import "./page.css";

  type ImageItem = {
    name: string;
    url: string;
    size: number;
    type: string;
    lastModified: number;
    source: "blob" | "file";
    path?: string;
    originalPath?: string;
    originalSize?: number;
    tempSessionDir?: string;       // どのtemp sessionの画像か
    originalArchivePath?: string;  // 展開元のアーカイブパス
  };



  let images = $state<ImageItem[]>([]);
  let currentIndex = $state(0);
  let zoom = $state(1);
  let fitToWindow = $state(true);
  let isDragging = $state(false);
  let fileInput = $state<HTMLInputElement | null>(null);
  let isLoading = $state(false);
  let isUpscaling = $state(false);
  let upscaleProvider = $state<"cpu" | "cuda" | "vulkan">("cpu");
  let statusMessage = $state("");
  let errorMessage = $state("");
  let dragCounter = 0;
  let imageReloadKey = $state(false);
  let pdfPage = $state(1);
  let pdfPageCount = $state(1);
  let lastIndex = -1;
  let readingDirection = $state<"ltr" | "rtl">("rtl");
  let spreadStartPage = $state(-1);
  let settingsReady = false;
  let lastSavedSnapshot: string | null = null;
  let currentItem = $derived(images[currentIndex] ?? null);
  let isPdf = $derived(
    currentItem
      ? currentItem.type === "application/pdf" || currentItem.name.toLowerCase().endsWith(".pdf")
      : false,
  );
  let spreadEnabled = $derived(spreadStartPage >= 1);
  let spreadStartIndex = $derived(
    spreadEnabled
      ? Math.min(images.length - 1, Math.max(0, spreadStartPage - 1))
      : 0,
  );
  let pdfSpreadStartPage = $derived(
    spreadEnabled
      ? Math.min(Math.max(1, spreadStartPage), Math.max(1, pdfPageCount))
      : 1,
  );
  let isBeforeSpreadStart = $derived(
    spreadEnabled && (isPdf ? pdfPage < pdfSpreadStartPage : currentIndex < spreadStartIndex),
  );
  let spreadMode = $derived(spreadEnabled && !isBeforeSpreadStart);
  let isFullscreen = $state(false);

  // 現在のフォルダ（セッション）内の位置と総数
  let sessionOffset = $derived((() => {
    const cur = images[currentIndex];
    if (!cur) return 0;
    const sd = cur.tempSessionDir;
    for (let i = 0; i < images.length; i++) {
      if (sd ? images[i].tempSessionDir === sd : (!images[i].tempSessionDir && images[i].type !== "archive/pending"))
        return i;
    }
    return 0;
  })());

  let sessionTotal = $derived((() => {
    const cur = images[currentIndex];
    if (!cur) return images.length;
    const sd = cur.tempSessionDir;
    return images.filter(img => sd ? img.tempSessionDir === sd : (!img.tempSessionDir && img.type !== "archive/pending")).length;
  })());
  function parseIni(text: string) {
    const result: Record<string, string> = {};
    for (const rawLine of text.split(/\r?\n/)) {
      const line = rawLine.trim();
      if (!line || line.startsWith(";") || line.startsWith("#")) continue;
      if (line.startsWith("[")) continue;
      const splitIndex = line.indexOf("=");
      if (splitIndex === -1) continue;
      const key = line.slice(0, splitIndex).trim();
      const value = line.slice(splitIndex + 1).trim();
      if (key) result[key] = value;
    }
    return result;
  }

  async function loadSettings() {
    try {
      console.debug("loadSettings: start");
      const text = await invoke<string | null>("load_settings");
      if (!text) return;
      const data = parseIni(text);
      if (data.spreadStartPage != null) {
        const parsed = Math.floor(Number(data.spreadStartPage));
        if (Number.isFinite(parsed)) {
          spreadStartPage = parsed >= 1 ? parsed : -1;
        }
      } else if (data.spreadMode != null) {
        spreadStartPage = data.spreadMode === "true" ? 1 : -1;
      }
      if (data.spreadStartPage != null) {
        const parsed = Math.floor(Number(data.spreadStartPage));
        if (Number.isFinite(parsed) && parsed >= 1) {
          spreadStartPage = parsed;
        }
      }
      if (data.readingDirection === "ltr" || data.readingDirection === "rtl") {
        readingDirection = data.readingDirection;
      }
      if (data.fitToWindow != null) {
        fitToWindow = data.fitToWindow === "true";
      }
      if (data.upscaleProvider === "cuda" || data.upscaleProvider === "vulkan") {
        upscaleProvider = data.upscaleProvider;
      }
      if (data.zoom != null) {
        const parsed = Number(data.zoom);
        if (Number.isFinite(parsed)) {
          zoom = parsed;
        }
      }
    } catch (error) {
      console.debug("loadSettings: failed", error);
    } finally {
      lastSavedSnapshot = JSON.stringify(getSettingsSnapshot());
      settingsReady = true;
    }
  }

  function getSettingsSnapshot() {
    return {
      spreadStartPage,
      readingDirection,
      fitToWindow,
      zoom: fitToWindow ? null : zoom,
      upscaleProvider,
    };
  }

  async function saveSettings() {
    try {
      const snapshot = JSON.stringify(getSettingsSnapshot());
      if (snapshot === lastSavedSnapshot) {
        return;
      }
      console.debug("saveSettings: start");
      const lines = [
        "[viewer]",
        `spreadStartPage=${spreadStartPage}`,
        `readingDirection=${readingDirection}`,
        `fitToWindow=${fitToWindow}`,
        `zoom=${zoom}`,
        `upscaleProvider=${upscaleProvider}`,
      ];
      await invoke("save_settings", { contents: lines.join("\n") });
      lastSavedSnapshot = snapshot;
      console.debug("saveSettings: done");
    } catch (error) {
      console.error("saveSettings: failed", error);
    }
  }

  function scheduleSaveSettings() {
    if (!settingsReady) return;
    void saveSettings();
  }


  function isArchiveFile(file: File) {
    const lowerName = file.name.toLowerCase();
    return (
      file.type === "application/zip" ||
      lowerName.endsWith(".zip") ||
      lowerName.endsWith(".cbz")
    );
  }

  function isRarFile(file: File) {
    const lowerName = file.name.toLowerCase();
    return (
      file.type === "application/x-rar-compressed" ||
      lowerName.endsWith(".rar")
    );
  }

  function isPdfFile(file: File) {
    const lowerName = file.name.toLowerCase();
    return file.type === "application/pdf" || lowerName.endsWith(".pdf");
  }

  function isPdfName(name: string) {
    return name.toLowerCase().endsWith(".pdf");
  }

  type FolderEntry = {
    path: string;
    name: string;
    entryType: string; // "image", "zip", "rar", "pdf"
    size: number;
  };

  function makeImageItemFromPath(path: string, name: string, size = 0): ImageItem {
    return { name, url: convertFileSrc(path), size, type: "image/*", lastModified: Date.now(), source: "file", path };
  }

  // FolderEntry 1つを処理して images に追加
  function processEntry(entry: FolderEntry) {
    if (entry.entryType === "image") {
      images = [...images, makeImageItemFromPath(entry.path, entry.name, entry.size)];
      isLoading = false;
    } else if (entry.entryType === "pdf") {
      images = [...images, { name: entry.name, url: convertFileSrc(entry.path), size: entry.size, type: "application/pdf", lastModified: Date.now(), source: "file", path: entry.path }];
      isLoading = false;
    } else if (entry.entryType === "zip" || entry.entryType === "rar") {
      // ZIP = フォルダ: プレースホルダーとして登録（ナビゲート時に遅延展開）
      images = [...images, { name: entry.name, url: "", size: entry.size, type: "archive/pending", lastModified: Date.now(), source: "file", path: entry.path }];
    }
  }

  async function extractToTemp(archivePath: string): Promise<string> {
    const channel = new Channel<{ current: number; total: number }>();
    channel.onmessage = ({ current, total }) => {
      const base = statusMessage.replace(/ \(\d+\/[\d?]+\)$/, "");
      statusMessage = total > 0 ? `${base} (${current}/${total})` : `${base} (${current}/?)`;
    };
    return invoke<string>("extract_to_temp", { archivePath, channel });
  }

  let extractingPending = false;
  let activeTempDir: string | null = null; // 現在表示中のセッション（temp dir）

  // currentIndex がプレースホルダーを指したとき遅延展開
  $effect(() => {
    const item = images[currentIndex];
    if (item?.type === "archive/pending" && item.path && !extractingPending) {
      void expandPendingArchive(currentIndex, item.path);
    }
  });

  async function expandPendingArchive(index: number, archivePath: string) {
    if (extractingPending) return;
    extractingPending = true;
    isLoading = true;
    statusMessage = `Extracting ${images[index]?.name ?? ""}...`;
    if (!sessionCleared) {
      await invoke("clear_session").catch(() => {});
      sessionCleared = true;
    }
    try {
      // ZIP = フォルダ: 展開してフォルダとして scan_directory で処理
      const folderPath = await extractToTemp(archivePath);
      const newSessionDir = folderPath.replace(/[\\/][^\\/]+$/, '');

      let localIndex = index;

      // 前のセッションがあれば、その画像をプレースホルダーに戻してからtempを削除
      if (activeTempDir && activeTempDir !== newSessionDir) {
        let firstOld = -1, lastOld = -1, oldArchivePath = '';
        for (let i = 0; i < images.length; i++) {
          if (images[i].tempSessionDir === activeTempDir) {
            if (firstOld < 0) firstOld = i;
            lastOld = i;
            oldArchivePath = images[i].originalArchivePath ?? '';
          }
        }
        if (firstOld >= 0 && oldArchivePath) {
          const oldName = oldArchivePath.split(/[\\/]/).pop() ?? '';
          const restored: ImageItem = { name: oldName, url: '', size: 0, type: 'archive/pending', lastModified: Date.now(), source: 'file', path: oldArchivePath };
          images = [...images.slice(0, firstOld), restored, ...images.slice(lastOld + 1)];
          // プレースホルダー1枚に置き換えたので、それ以降のインデックスを調整
          if (firstOld < localIndex) {
            localIndex -= (lastOld - firstOld);
          }
        }
        await invoke("delete_single_temp_dir", { path: activeTempDir }).catch(() => {});
      }

      activeTempDir = newSessionDir;
      const entries = await invoke<FolderEntry[]>("scan_directory", { path: folderPath }).catch(() => [] as FolderEntry[]);
      const expanded: ImageItem[] = [];
      for (const entry of entries) {
        if (entry.entryType === "image") {
          expanded.push({ ...makeImageItemFromPath(entry.path, entry.name, entry.size), tempSessionDir: newSessionDir, originalArchivePath: archivePath });
        } else if (entry.entryType === "pdf") {
          expanded.push({ name: entry.name, url: convertFileSrc(entry.path), size: entry.size, type: "application/pdf", lastModified: Date.now(), source: "file", path: entry.path, tempSessionDir: newSessionDir, originalArchivePath: archivePath });
        } else if (entry.entryType === "zip" || entry.entryType === "rar") {
          expanded.push({ name: entry.name, url: "", size: entry.size, type: "archive/pending", lastModified: Date.now(), source: "file", path: entry.path });
        }
      }
      images = [...images.slice(0, localIndex), ...expanded, ...images.slice(localIndex + 1)];
      // 新フォルダの先頭から表示
      currentIndex = expanded.length > 0 ? localIndex : Math.min(localIndex, images.length - 1);
    } catch (error) {
      console.error(error);
      images = [...images.slice(0, index), ...images.slice(index + 1)];
      if (currentIndex >= images.length) currentIndex = Math.max(0, images.length - 1);
    } finally {
      extractingPending = false;
      isLoading = false;
      statusMessage = "";
    }
  }

  // パスを処理: ファイルならそのまま、フォルダなら scan_directory して各エントリを処理
  async function processPath(path: string) {
    const lower = path.toLowerCase();
    if (/\.(png|jpg|jpeg|webp|gif|svg|bmp|avif)$/.test(lower)) {
      const name = path.split(/[\\/]/).pop() ?? path;
      images = [...images, makeImageItemFromPath(path, name)];
      isLoading = false;
    } else if (/\.pdf$/.test(lower)) {
      const name = path.split(/[\\/]/).pop() ?? path;
      images = [...images, { name, url: convertFileSrc(path), size: 0, type: "application/pdf", lastModified: Date.now(), source: "file", path }];
      isLoading = false;
    } else if (/\.(zip|cbz|rar)$/.test(lower)) {
      // ZIP/RAR → tempに展開 → フォルダとして scan_directory で処理（ZIP = フォルダ）
      const name = path.split(/[\\/]/).pop() ?? path;
      statusMessage = `Extracting ${name}...`;
      if (!sessionCleared) {
        await invoke("clear_session").catch(() => {});
        sessionCleared = true;
      }
      const folderPath = await extractToTemp(path);
      // 展開後のフォルダをフォルダとして扱う（内側ZIPはプレースホルダーになる）
      const entries = await invoke<FolderEntry[]>("scan_directory", { path: folderPath }).catch(() => [] as FolderEntry[]);
      for (const entry of entries) {
        processEntry(entry);
      }
      isLoading = false;
    } else {
      // フォルダ → scan_directory して各エントリを順次処理
      const entries = await invoke<FolderEntry[]>("scan_directory", { path }).catch(() => [] as FolderEntry[]);
      for (const entry of entries) {
        processEntry(entry);
      }
    }
  }

  let sessionCleared = false;

  async function addFilesByPaths(paths: string[]) {
    if (!paths.length) return;

    // 既存画像をクリア → 前セッションのtempを削除（ロックなし確認済み）
    for (const img of images) { if (img.source === "blob") URL.revokeObjectURL(img.url); }
    images = [];
    currentIndex = 0;
    activeTempDir = null;
    await invoke("clear_session").catch(() => {});
    sessionCleared = true; // 今セッションではもう呼ばない

    isLoading = true;
    statusMessage = "";
    errorMessage = "";

    try {
      for (const path of paths) {
        await processPath(path);
      }
    } catch (error) {
      console.error(error);
      errorMessage = "Failed to process files.";
    } finally {
      isLoading = false;
      statusMessage = "";
    }
  }

  async function addDroppedPaths(paths: string[]) {
    await addFilesByPaths(paths);
  }

  async function addFiles(fileList: FileList | null) {
    if (!fileList) return;
    const newItems: ImageItem[] = [];
    let shouldReplace = false;

    isLoading = true;
    statusMessage = "";
    errorMessage = "";

    try {
      for (const file of Array.from(fileList)) {
        if (file.type.startsWith("image/")) {
          newItems.push({
            name: file.name,
            url: URL.createObjectURL(file),
            size: file.size,
            type: file.type,
            lastModified: file.lastModified,
            source: "blob",
          });
          continue;
        }

        if (isPdfFile(file)) {
          newItems.push({
            name: file.name,
            url: URL.createObjectURL(file),
            size: file.size,
            type: "application/pdf",
            lastModified: file.lastModified,
            source: "blob",
          });
          continue;
        }

        if (isArchiveFile(file) || isRarFile(file)) {
          // アーカイブはバイト経由では処理できないためスキップ
          // （Tauri ダイアログかネイティブドロップ経由で処理される）
          continue;
        }
      }
    } catch (error) {
      console.error(error);
      errorMessage = "Failed to extract archive.";
    } finally {
      isLoading = false;
      statusMessage = "";
    }

    if (!newItems.length) return;
    if (shouldReplace) {
      for (const image of images) {
        if (image.source === "blob") {
          URL.revokeObjectURL(image.url);
        }
      }
      images = [...newItems];
      currentIndex = 0;
      return;
    }
    const startIndex = images.length;
    images = [...images, ...newItems];
    if (startIndex === 0) {
      currentIndex = 0;
    } else {
      currentIndex = startIndex;
    }
  }

  async function handleFileChange(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    await addFiles(input.files);
    input.value = "";
  }

  async function handleDrop(event: DragEvent) {
    event.preventDefault();
    isDragging = false;
    await addFiles(event.dataTransfer?.files ?? null);
  }

  function handleDragOver(event: DragEvent) {
    event.preventDefault();
    if (event.dataTransfer) {
      event.dataTransfer.dropEffect = "copy";
    }
    isDragging = true;
  }

  function handleDragEnter(event: DragEvent) {
    event.preventDefault();
    dragCounter += 1;
    isDragging = true;
  }

  function handleDragLeave(event: DragEvent) {
    event.preventDefault();
    dragCounter = Math.max(0, dragCounter - 1);
    if (dragCounter === 0) {
      isDragging = false;
    }
  }

  async function handleWindowDrop(event: DragEvent) {
    event.preventDefault();
    dragCounter = 0;
    isDragging = false;
    const files = event.dataTransfer?.files ?? null;
    if (files && files.length) {
      await addFiles(files);
    }
  }

  function revertToOriginal() {
    const item = images[currentIndex];
    if (!item?.originalPath) return;
    const newImages = [...images];
    newImages[currentIndex] = {
      ...item,
      url: convertFileSrc(item.originalPath),
      path: item.originalPath,
      size: item.originalSize ?? item.size,
      name: item.originalPath.split(/[\\/]/).pop() ?? item.name,
      originalPath: undefined,
      originalSize: undefined,
    };
    images = newImages;
    fitToWindow = true;
  }

  function toggleUpscaleProvider() {
    if (upscaleProvider === "cpu") upscaleProvider = "cuda";
    else if (upscaleProvider === "cuda") upscaleProvider = "vulkan";
    else upscaleProvider = "cpu";
  }

  async function upscaleCurrentImage(scale: 2 | 4 = 2) {
    const item = images[currentIndex];
    const inputPath = item?.originalPath ?? item?.path;
    if (!item || !inputPath) {
      errorMessage = "Only local file images can be upscaled.";
      return;
    }
    if (isUpscaling) return;
    isUpscaling = true;
    errorMessage = "";
    try {
      const newImages = [...images];
      const suffix = `_${scale}x.png`;
      const originalPath = item.originalPath ?? item.path!;
      const originalSize = item.originalSize ?? item.size;

      function nameFromOriginal(origPath: string): string {
        const fileName = origPath.split(/[\\/]/).pop() ?? "";
        return fileName.replace(/\.[^.]+$/, "") + suffix;
      }

      const result = await invoke<{ path: string; size: number }>("upscale_image", { inputPath, scale, provider: upscaleProvider });
      newImages[currentIndex] = {
        name: nameFromOriginal(originalPath),
        url: convertFileSrc(result.path),
        size: result.size,
        type: "image/*",
        lastModified: Date.now(),
        source: "file",
        path: result.path,
        originalPath,
        originalSize,
      };

      const secondItem = spreadMode ? images[currentIndex + 1] : null;
      const secondInputPath = secondItem?.originalPath ?? secondItem?.path;
      if (secondItem && secondInputPath) {
        const secondOriginalPath = secondItem.originalPath ?? secondItem.path!;
        const secondOriginalSize = secondItem.originalSize ?? secondItem.size;
        const result2 = await invoke<{ path: string; size: number }>("upscale_image", { inputPath: secondInputPath, scale, provider: upscaleProvider });
        newImages[currentIndex + 1] = {
          name: nameFromOriginal(secondOriginalPath),
          url: convertFileSrc(result2.path),
          size: result2.size,
          type: "image/*",
          lastModified: Date.now(),
          source: "file",
          path: result2.path,
          originalPath: secondOriginalPath,
          originalSize: secondOriginalSize,
        };
      }

      images = newImages;
      zoom = zoom / scale;
    } catch (error) {
      if (error === "NCNN_NOT_INSTALLED") {
        const ok = confirm("GPU (Vulkan) upscaling requires additional files (~46MB).\nDownload now?");
        if (ok) {
          try {
            statusMessage = "Downloading...";
            isUpscaling = true;
            await invoke("download_ncnn_vulkan");
            isUpscaling = false;
            statusMessage = "";
            void upscaleCurrentImage(scale);
          } catch (dlError) {
            console.error(dlError);
            errorMessage = "Download failed.";
          }
        }
      } else {
        console.error(error);
        errorMessage = "Upscaling failed.";
      }
    } finally {
      isUpscaling = false;
    }
  }

  function clearImages() {
    for (const image of images) {
      if (image.source === "blob") {
        URL.revokeObjectURL(image.url);
      }
    }
    images = [];
    currentIndex = 0;
    zoom = 1;
    fitToWindow = true;
    pdfPage = 1;
    pdfPageCount = 1;
  }

  function getLastImageStartIndex() {
    if (!images.length) return 0;
    return images.length - 1;
  }

  function getLastPdfStartPage() {
    return pdfPageCount;
  }

  function prevImage() {
    if (!images.length) return;
    const step = spreadMode ? 2 : 1;
    currentIndex = Math.max(0, currentIndex - step);
  }

  function nextImage() {
    if (!images.length) return;
    const step = spreadMode ? 2 : 1;
    const lastIndex = getLastImageStartIndex();
    currentIndex = Math.min(lastIndex, currentIndex + step);
  }

  function zoomIn() {
    fitToWindow = false;
    zoom = Math.min(5, Number((zoom + 0.1).toFixed(2)));
  }

  function zoomOut() {
    fitToWindow = false;
    zoom = Math.max(0.2, Number((zoom - 0.1).toFixed(2)));
  }

  function toggleFit() {
    if (fitToWindow) {
      fitToWindow = false;
      zoom = 1;
    } else {
      fitToWindow = true;
    }
  }

  function reloadCurrentImage() {
    if (!images.length) return;
    imageReloadKey = !imageReloadKey;
  }

  function setPdfPageCount(count: number) {
    pdfPageCount = Math.max(1, count);
    const lastPage = getLastPdfStartPage();
    pdfPage = Math.min(pdfPage, lastPage);
    if (spreadEnabled) {
      alignSpreadStart();
    }
  }

  function prevPdfPage() {
    const step = spreadMode ? 2 : 1;
    pdfPage = Math.max(1, pdfPage - step);
  }

  function nextPdfPage() {
    const step = spreadMode ? 2 : 1;
    const lastPage = getLastPdfStartPage();
    pdfPage = Math.min(lastPage, pdfPage + step);
  }

  function toggleSpreadMode() {
    if (spreadStartPage >= 1) {
      spreadStartPage = -1;
      return;
    }
    spreadStartPage = isPdf ? pdfPage : currentIndex + 1;
    alignSpreadStart();
  }

  function alignSpreadStart() {
    if (!spreadEnabled) return;

    if (isPdf) {
      if (pdfPageCount <= 0) return;
      const maxPage = Math.max(1, pdfPageCount);
      if (spreadStartPage > maxPage) {
        spreadStartPage = maxPage;
      }
      const start = Math.max(1, spreadStartPage);
      if (pdfPage < start) {
        return;
      }
      const offset = (pdfPage - start) % 2;
      if (offset === 1) {
        pdfPage = Math.max(1, pdfPage - 1);
      }
      return;
    }

    if (!images.length) return;
    const maxIndex = images.length - 1;
    const startIndex = Math.min(maxIndex, Math.max(0, spreadStartPage - 1));
    if (currentIndex < startIndex) {
      return;
    }
    const offset = (currentIndex - startIndex) % 2;
    if (offset === 1) {
      currentIndex = Math.max(0, currentIndex - 1);
    }
  }

  function toggleReadingDirection() {
    readingDirection = readingDirection === "ltr" ? "rtl" : "ltr";
  }

  async function toggleFullscreen() {
    try {
      const next = await invoke<boolean>("toggle_fullscreen");
      isFullscreen = next;
    } catch (error) {
      console.error(error);
      errorMessage = "Failed to toggle fullscreen.";
    }
  }

  function handlePdfError(message: string) {
    errorMessage = message;
  }

  function updatePdfFitZoom(nextZoom: number) {
    if (!fitToWindow) return;
    if (!Number.isFinite(nextZoom)) return;
    const rounded = Number(nextZoom.toFixed(4));
    if (Math.abs(zoom - rounded) < 0.0005) return;
    zoom = rounded;
  }

  function handleResize() {
    reloadCurrentImage();
  }

  function openPicker() {
    void openDialog({
      multiple: true,
      filters: [{ name: "Supported Files", extensions: ["png","jpg","jpeg","webp","gif","svg","bmp","avif","zip","cbz","rar","pdf"] }],
    }).then((result) => {
      if (!result) return;
      const paths = Array.isArray(result) ? result : [result];
      void addFilesByPaths(paths);
    });
  }

  function handleDropzoneKey(event: KeyboardEvent) {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      openPicker();
    }
  }

  function handleGlobalKey(event: KeyboardEvent) {
    const target = event.target as HTMLElement | null;
    const tag = target?.tagName?.toLowerCase();
    if (tag === "input" || tag === "textarea" || target?.isContentEditable) {
      return;
    }

    switch (event.key) {
      case "s":
      case "S":
        event.preventDefault();
        toggleSpreadMode();
        break;
      case "F11":
        event.preventDefault();
        void toggleFullscreen();
        break;
      case "2":
        event.preventDefault();
        toggleReadingDirection();
        break;
      case "ArrowLeft":
        event.preventDefault();
        if (readingDirection === "ltr") {
          if (isPdf) {
            prevPdfPage();
          } else {
            prevImage();
          }
        } else if (isPdf) {
          nextPdfPage();
        } else {
          nextImage();
        }
        break;
      case "ArrowRight":
        event.preventDefault();
        if (readingDirection === "ltr") {
          if (isPdf) {
            nextPdfPage();
          } else {
            nextImage();
          }
        } else if (isPdf) {
          prevPdfPage();
        } else {
          prevImage();
        }
        break;
      case "Home":
        event.preventDefault();
        currentIndex = 0;
        break;
      case "End":
        event.preventDefault();
        if (images.length) {
          currentIndex = images.length - 1;
        }
        break;
      default:
        break;
    }
  }

  onDestroy(() => {
    for (const image of images) {
      if (image.source === "blob") {
        URL.revokeObjectURL(image.url);
      }
    }
    void saveSettings();
  });

  $effect(() => {
    if (currentIndex !== lastIndex) {
      lastIndex = currentIndex;
      pdfPage = 1;
      pdfPageCount = 1;
    }
  });

  $effect(() => {
    spreadStartPage;
    readingDirection;
    fitToWindow;
    if (!fitToWindow) {
      zoom;
    }
    scheduleSaveSettings();
  });

  $effect(() => {
    spreadStartPage;
    spreadMode;
    currentIndex;
    images.length;
    pdfPage;
    pdfPageCount;
    isPdf;
    alignSpreadStart();
  });



  onMount(() => {
    void loadSettings();
    const preventDefaults = (event: DragEvent) => {
      event.preventDefault();
    };
    window.addEventListener("dragover", preventDefaults);
    window.addEventListener("drop", handleWindowDrop);
    window.addEventListener("dragenter", handleDragEnter);
    window.addEventListener("dragleave", handleDragLeave);
    window.addEventListener("keydown", handleGlobalKey);
    let unlistenDragDrop: (() => void) | null = null;
    const appWindow = getCurrentWindow();
    appWindow
      .onDragDropEvent(async (event) => {
        const payload = event.payload;
        if (payload.type === "over") {
          isDragging = true;
          return;
        }
        if (payload.type === "drop") {
          dragCounter = 0;
          isDragging = false;
          await addDroppedPaths(payload.paths ?? []);
          return;
        }
        dragCounter = 0;
        isDragging = false;
      })
      .then((fn) => {
        unlistenDragDrop = fn;
      });
    const handleBeforeUnload = () => {
      void saveSettings();
    };
    window.addEventListener("beforeunload", handleBeforeUnload);

    return () => {
      window.removeEventListener("dragover", preventDefaults);
      window.removeEventListener("drop", handleWindowDrop);
      window.removeEventListener("dragenter", handleDragEnter);
      window.removeEventListener("dragleave", handleDragLeave);
      window.removeEventListener("keydown", handleGlobalKey);
      unlistenDragDrop?.();
      window.removeEventListener("beforeunload", handleBeforeUnload);
    };
  });
</script>

<svelte:window on:resize={handleResize} />

<PageView
  bind:fileInput
  bind:zoom
  {imageReloadKey}
  {images}
  {currentIndex}
  {fitToWindow}
  {isDragging}
  {isLoading}
  {isUpscaling}
  {statusMessage}
  {errorMessage}
  {pdfPage}
  {pdfPageCount}
  {spreadStartPage}
  {spreadMode}
  {spreadEnabled}
  {readingDirection}
  {handleFileChange}
  {handleDropzoneKey}
  {handleDragEnter}
  {handleDragOver}
  {handleDragLeave}
  {handleDrop}
  {prevImage}
  {nextImage}
  {zoomIn}
  {zoomOut}
  {toggleFit}
  {setPdfPageCount}
  {prevPdfPage}
  {nextPdfPage}
  {handlePdfError}
  {updatePdfFitZoom}
  {toggleSpreadMode}
  {toggleReadingDirection}
  {toggleFullscreen}
  {upscaleCurrentImage}
  {upscaleProvider}
  {toggleUpscaleProvider}
  {revertToOriginal}
  {sessionOffset}
  {sessionTotal}
/>
