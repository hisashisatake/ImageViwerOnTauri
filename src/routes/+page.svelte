<script lang="ts">
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
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
  };

  type ExtractedFile = {
    path: string;
    name: string;
    size: number;
  };

  let images = $state<ImageItem[]>([]);
  let currentIndex = $state(0);
  let zoom = $state(1);
  let fitToWindow = $state(true);
  let isDragging = $state(false);
  let fileInput = $state<HTMLInputElement | null>(null);
  let isLoading = $state(false);
  let statusMessage = $state("");
  let errorMessage = $state("");
  let dragCounter = 0;
  let imageReloadKey = $state(false);
  let pdfPage = $state(1);
  let pdfPageCount = $state(1);
  let lastIndex = -1;
  let readingDirection = $state<"ltr" | "rtl">("rtl");
  let spreadStartPage = $state(-1);
  let saveTimer: ReturnType<typeof setTimeout> | null = null;
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
      if (data.zoom != null) {
        const parsed = Number(data.zoom);
        if (Number.isFinite(parsed)) {
          zoom = parsed;
        }
      }
    } catch (error) {
      console.debug("loadSettings: failed", error);
    }
  }

  async function saveSettings() {
    try {
      console.debug("saveSettings: start");
      const lines = [
        "[viewer]",
        `spreadStartPage=${spreadStartPage}`,
        `readingDirection=${readingDirection}`,
        `fitToWindow=${fitToWindow}`,
        `zoom=${zoom}`,
      ];
      await invoke("save_settings", { contents: lines.join("\n") });
      console.debug("saveSettings: done");
    } catch (error) {
      console.error("saveSettings: failed", error);
    }
  }

  function scheduleSaveSettings() {
    if (saveTimer) {
      clearTimeout(saveTimer);
    }
    saveTimer = setTimeout(() => {
      saveTimer = null;
      void saveSettings();
    }, 300);
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

  async function extractArchive(file: File): Promise<ImageItem[]> {
    const bytes = new Uint8Array(await file.arrayBuffer());
    const extracted = await invoke<ExtractedFile[]>("extract_archive", {
      archiveName: file.name,
      bytes,
    });
    return extracted.map((item) => ({
      name: item.name,
      url: convertFileSrc(item.path),
      size: item.size,
      type: "image/*",
      lastModified: Date.now(),
      source: "file",
    }));
  }

  async function extractRar(file: File): Promise<ImageItem[]> {
    const bytes = new Uint8Array(await file.arrayBuffer());
    const extracted = await invoke<ExtractedFile[]>("extract_rar", {
      archiveName: file.name,
      bytes,
    });
    return extracted.map((item) => ({
      name: item.name,
      url: convertFileSrc(item.path),
      size: item.size,
      type: "image/*",
      lastModified: Date.now(),
      source: "file",
    }));
  }

  async function addDroppedPaths(paths: string[]) {
    if (!paths.length) return;
    isLoading = true;
    statusMessage = "";
    errorMessage = "";
    const shouldReplace = paths.some((path) => {
      const lower = path.toLowerCase();
      return lower.endsWith(".zip") || lower.endsWith(".cbz") || lower.endsWith(".rar");
    });

    try {
      const extracted = await invoke<ExtractedFile[]>("handle_file_drop", { paths });
      const newItems = extracted.map((item) => ({
        name: item.name,
        url: convertFileSrc(item.path),
        size: item.size,
        type: isPdfName(item.name) ? "application/pdf" : "image/*",
        lastModified: Date.now(),
        source: "file" as const,
      }));
      if (newItems.length) {
        if (shouldReplace) {
          for (const image of images) {
            if (image.source === "blob") {
              URL.revokeObjectURL(image.url);
            }
          }
          images = [...newItems];
          currentIndex = 0;
        } else {
          const startIndex = images.length;
          images = [...images, ...newItems];
          if (startIndex === 0) {
            currentIndex = 0;
          } else {
            currentIndex = startIndex;
          }
        }
      }
    } catch (error) {
      console.error(error);
      errorMessage = "Failed to read dropped files.";
    } finally {
      isLoading = false;
      statusMessage = "";
    }
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

        if (isArchiveFile(file)) {
          statusMessage = `Extracting ${file.name}...`;
          const extracted = await extractArchive(file);
          newItems.push(...extracted);
          shouldReplace = true;
          continue;
        }

        if (isRarFile(file)) {
          statusMessage = `Extracting ${file.name}...`;
          const extracted = await extractRar(file);
          newItems.push(...extracted);
          shouldReplace = true;
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
    fileInput?.click();
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
    spreadMode;
    readingDirection;
    fitToWindow;
    zoom;
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
  {statusMessage}
  {errorMessage}
  {pdfPage}
  {pdfPageCount}
  {spreadStartPage}
  {spreadMode}
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
/>
