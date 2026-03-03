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

  function isArchiveFile(file: File) {
    const lowerName = file.name.toLowerCase();
    return (
      file.type === "application/zip" ||
      lowerName.endsWith(".zip") ||
      lowerName.endsWith(".cbz")
    );
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

  async function addDroppedPaths(paths: string[]) {
    if (!paths.length) return;
    isLoading = true;
    statusMessage = "";
    errorMessage = "";

    try {
      const extracted = await invoke<ExtractedFile[]>("handle_file_drop", { paths });
      const newItems = extracted.map((item) => ({
        name: item.name,
        url: convertFileSrc(item.path),
        size: item.size,
        type: "image/*",
        lastModified: Date.now(),
        source: "file" as const,
      }));
      if (newItems.length) {
        images = [...images, ...newItems];
        if (images.length === newItems.length) {
          currentIndex = 0;
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

        if (isArchiveFile(file)) {
          statusMessage = `Extracting ${file.name}...`;
          const extracted = await extractArchive(file);
          newItems.push(...extracted);
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
    images = [...images, ...newItems];
    if (images.length === newItems.length) {
      currentIndex = 0;
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
  }

  function prevImage() {
    if (!images.length) return;
    currentIndex = Math.max(0, currentIndex - 1);
  }

  function nextImage() {
    if (!images.length) return;
    currentIndex = Math.min(images.length - 1, currentIndex + 1);
  }

  function zoomIn() {
    fitToWindow = false;
    zoom = Math.min(5, Number((zoom + 0.1).toFixed(2)));
  }

  function zoomOut() {
    fitToWindow = false;
    zoom = Math.max(0.2, Number((zoom - 0.1).toFixed(2)));
  }

  function resetZoom() {
    zoom = 1;
  }

  function toggleFit() {
    fitToWindow = !fitToWindow;
  }

  function reloadCurrentImage() {
    if (!images.length) return;
    imageReloadKey = !imageReloadKey;
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
      case "ArrowLeft":
        event.preventDefault();
        prevImage();
        break;
      case "ArrowRight":
        event.preventDefault();
        nextImage();
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
  });

  onMount(() => {
    const preventDefaults = (event: DragEvent) => {
      event.preventDefault();
    };
    window.addEventListener("dragover", preventDefaults);
    window.addEventListener("drop", handleWindowDrop);
    window.addEventListener("dragenter", handleDragEnter);
    window.addEventListener("dragleave", handleDragLeave);
    window.addEventListener("keydown", handleGlobalKey);
    let unlistenDragDrop: (() => void) | null = null;
    getCurrentWindow()
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

    return () => {
      window.removeEventListener("dragover", preventDefaults);
      window.removeEventListener("drop", handleWindowDrop);
      window.removeEventListener("dragenter", handleDragEnter);
      window.removeEventListener("dragleave", handleDragLeave);
      window.removeEventListener("keydown", handleGlobalKey);
      unlistenDragDrop?.();
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
  {handleFileChange}
  {clearImages}
  {openPicker}
  {handleDropzoneKey}
  {handleDragEnter}
  {handleDragOver}
  {handleDragLeave}
  {handleDrop}
  {prevImage}
  {nextImage}
  {zoomIn}
  {zoomOut}
  {resetZoom}
  {toggleFit}
/>
