<script lang="ts">
  import { onDestroy, onMount } from "svelte";

  type ImageItem = {
    name: string;
    url: string;
    size: number;
    type: string;
    lastModified: number;
    source: "blob" | "file";
  };

  export let images: ImageItem[];
  export let currentIndex: number;
  export let zoom: number;
  export let fitToWindow: boolean;
  export let isDragging: boolean;
  export let isLoading: boolean;
  export let statusMessage: string;
  export let errorMessage: string;
  export let fileInput: HTMLInputElement | null;
  export let imageReloadKey: boolean;

  export let handleFileChange: (event: Event) => void | Promise<void>;
  export let clearImages: () => void;
  export let openPicker: () => void;
  export let handleDropzoneKey: (event: KeyboardEvent) => void;
  export let handleDragEnter: (event: DragEvent) => void;
  export let handleDragOver: (event: DragEvent) => void;
  export let handleDragLeave: (event: DragEvent) => void;
  export let handleDrop: (event: DragEvent) => void | Promise<void>;
  export let prevImage: () => void;
  export let nextImage: () => void;
  export let zoomIn: () => void;
  export let zoomOut: () => void;
  export let resetZoom: () => void;
  export let toggleFit: () => void;

  let canvasEl: HTMLDivElement | null = null;
  let imgEl: HTMLImageElement | null = null;
  let resizeObserver: ResizeObserver | null = null;

  function updateFitZoom() {
    if (!fitToWindow) return;
    if (!canvasEl || !imgEl) return;
    if (!imgEl.naturalHeight) return;
    const canvasHeight = canvasEl.getBoundingClientRect().height;
    if (!canvasHeight) return;
    const nextZoom = canvasHeight / imgEl.naturalHeight;
    zoom = Number(nextZoom.toFixed(4));
  }

  onMount(() => {
    resizeObserver = new ResizeObserver(() => {
      updateFitZoom();
    });
    if (canvasEl) {
      resizeObserver.observe(canvasEl);
    }

    return () => {
      resizeObserver?.disconnect();
      resizeObserver = null;
    };
  });

  onDestroy(() => {
    resizeObserver?.disconnect();
    resizeObserver = null;
  });

  $: if (fitToWindow && images[currentIndex]) {
    updateFitZoom();
  }
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
    {#if isLoading}
      <div class="loading-overlay" role="status" aria-live="polite">
        <div class="spinner" aria-hidden="true"></div>
        <span>{statusMessage || "Loading..."}</span>
      </div>
    {/if}
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
        <div class="canvas" bind:this={canvasEl}>
          {#if images[currentIndex]}
            {#key imageReloadKey}
              <img
                bind:this={imgEl}
                src={images[currentIndex].url}
                alt={images[currentIndex].name}
                onload={updateFitZoom}
                style={`transform: translate(-50%, -50%) scale(${zoom});`}
              />
            {/key}
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
        {#if errorMessage}
          <p>{errorMessage}</p>
        {/if}
      </div>
    {/if}
  </div>
</main>
