<script lang="ts">
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onDestroy, onMount } from "svelte";
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
  let fileInput: HTMLInputElement | null = null;
  let isLoading = $state(false);
  let statusMessage = $state("");
  let errorMessage = $state("");
  let dragCounter = 0;

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

  function openPicker() {
    fileInput?.click();
  }

  function handleDropzoneKey(event: KeyboardEvent) {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      openPicker();
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
      unlistenDragDrop?.();
    };
  });
</script>

<main class="app">
  <header class="toolbar">
    <div>
      <h1>Image Viewer</h1>
      <p>Drag & drop images or use the picker.</p>
    </div>
    <div class="actions">
      <label class="button primary">
        <input
          bind:this={fileInput}
          type="file"
          multiple
          accept="image/*,.zip,application/zip"
          onchange={handleFileChange}
        />
        Add Images
      </label>
      <button class="button" onclick={clearImages} disabled={!images.length}>Clear</button>
    </div>
  </header>

  <div
    class:dragging={isDragging}
    class="dropzone"
    role="button"
    tabindex="0"
    aria-label="Image dropzone. Click to open file picker."
    onclick={openPicker}
    onkeydown={handleDropzoneKey}
    ondragenter={handleDragEnter}
    ondragover={handleDragOver}
    ondragleave={handleDragLeave}
    ondrop={handleDrop}
  >
    {#if images.length}
      <div class="viewer">
        <div class="viewer-toolbar">
          <button
            class="button"
            onclick={(event) => {
              event.stopPropagation();
              prevImage();
            }}
            disabled={currentIndex === 0}
          >
            Prev
          </button>
          <span class="counter">{currentIndex + 1} / {images.length}</span>
          <button
            class="button"
            onclick={(event) => {
              event.stopPropagation();
              nextImage();
            }}
            disabled={currentIndex === images.length - 1}
          >
            Next
          </button>
          <div class="spacer"></div>
          <button
            class="button"
            onclick={(event) => {
              event.stopPropagation();
              zoomOut();
            }}
          >
            -
          </button>
          <span class="zoom">{Math.round(zoom * 100)}%</span>
          <button
            class="button"
            onclick={(event) => {
              event.stopPropagation();
              zoomIn();
            }}
          >
            +
          </button>
          <button
            class="button"
            onclick={(event) => {
              event.stopPropagation();
              resetZoom();
            }}
          >
            Reset
          </button>
          <button
            class="button"
            onclick={(event) => {
              event.stopPropagation();
              toggleFit();
            }}
          >
            {fitToWindow ? "Actual Size" : "Fit"}
          </button>
        </div>
        <div class="canvas">
          {#if images[currentIndex]}
            <img
              src={images[currentIndex].url}
              alt={images[currentIndex].name}
              class:fit={fitToWindow}
              style={!fitToWindow ? `transform: scale(${zoom});` : ""}
            />
          {/if}
        </div>
        <div class="meta">
          <span>{images[currentIndex].name}</span>
          <span>{Math.round(images[currentIndex].size / 1024)} KB</span>
        </div>
      </div>
    {:else}
      <div class="empty">
        <p>Drop image files here.</p>
        <p>Supported: png, jpg, webp, gif, svg, zip.</p>
        {#if isLoading}
          <p>{statusMessage || "Loading..."}</p>
        {:else if errorMessage}
          <p>{errorMessage}</p>
        {/if}
      </div>
    {/if}
  </div>
</main>
