<script lang="ts">
  import { onDestroy } from "svelte";
  import "./page.css";

  type ImageItem = {
    name: string;
    url: string;
    size: number;
    type: string;
    lastModified: number;
  };

  let images = $state<ImageItem[]>([]);
  let currentIndex = $state(0);
  let zoom = $state(1);
  let fitToWindow = $state(true);
  let isDragging = $state(false);
  let fileInput: HTMLInputElement | null = null;

  function addFiles(fileList: FileList | null) {
    if (!fileList) return;
    const newItems: ImageItem[] = [];

    for (const file of Array.from(fileList)) {
      if (!file.type.startsWith("image/")) continue;
      newItems.push({
        name: file.name,
        url: URL.createObjectURL(file),
        size: file.size,
        type: file.type,
        lastModified: file.lastModified,
      });
    }

    if (!newItems.length) return;
    images = [...images, ...newItems];
    if (images.length === newItems.length) {
      currentIndex = 0;
    }
  }

  function handleFileChange(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    addFiles(input.files);
    input.value = "";
  }

  function handleDrop(event: DragEvent) {
    event.preventDefault();
    isDragging = false;
    addFiles(event.dataTransfer?.files ?? null);
  }

  function handleDragOver(event: DragEvent) {
    event.preventDefault();
    isDragging = true;
  }

  function handleDragLeave(event: DragEvent) {
    event.preventDefault();
    isDragging = false;
  }

  function clearImages() {
    for (const image of images) {
      URL.revokeObjectURL(image.url);
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
      URL.revokeObjectURL(image.url);
    }
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
          accept="image/*"
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
    ondragover={handleDragOver}
    ondragleave={handleDragLeave}
    ondrop={handleDrop}
  >
    {#if images.length}
      <div class="viewer">
        <div class="viewer-toolbar">
          <button class="button" onclick={prevImage} disabled={currentIndex === 0}>Prev</button>
          <span class="counter">{currentIndex + 1} / {images.length}</span>
          <button class="button" onclick={nextImage} disabled={currentIndex === images.length - 1}>Next</button>
          <div class="spacer"></div>
          <button class="button" onclick={zoomOut}>-</button>
          <span class="zoom">{Math.round(zoom * 100)}%</span>
          <button class="button" onclick={zoomIn}>+</button>
          <button class="button" onclick={resetZoom}>Reset</button>
          <button class="button" onclick={toggleFit}>{fitToWindow ? "Actual Size" : "Fit"}</button>
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
        <p>Supported: png, jpg, webp, gif, svg.</p>
      </div>
    {/if}
  </div>
</main>
