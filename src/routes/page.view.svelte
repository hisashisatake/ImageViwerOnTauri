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
  let isFabOpen = false;

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

  function toggleFab() {
    isFabOpen = !isFabOpen;
  }

  function closeFab() {
    isFabOpen = false;
  }
</script>

<main class="app">
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
    <input
      class="file-input"
      bind:this={fileInput}
      type="file"
      multiple
      accept="image/*,.zip,application/zip"
      onchange={handleFileChange}
    />
    {#if isLoading}
      <div class="loading-overlay" role="status" aria-live="polite">
        <div class="spinner" aria-hidden="true"></div>
        <span>{statusMessage || "Loading..."}</span>
      </div>
    {/if}
    {#if images.length}
      <div class="viewer">
        <div class="canvas" bind:this={canvasEl}>
          <button
            class="nav-button left"
            onclick={(event) => {
              event.stopPropagation();
              prevImage();
            }}
            disabled={currentIndex === 0}
            aria-label="Previous image"
          >
            ‹
          </button>
          <button
            class="nav-button right"
            onclick={(event) => {
              event.stopPropagation();
              nextImage();
            }}
            disabled={currentIndex === images.length - 1}
            aria-label="Next image"
          >
            ›
          </button>
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
          <div class="meta">
            <span>{images[currentIndex].name}</span>
            <span>{Math.round(images[currentIndex].size / 1024)} KB</span>
            <span class="counter">{currentIndex + 1} / {images.length}</span>
            <span class="zoom">{Math.round(zoom * 100)}%</span>
          </div>
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

    <div class="fab">
      {#if isFabOpen}
        <div class="fab-menu" role="menu">
          <button
            class="fab-item"
            onclick={(event) => {
              event.stopPropagation();
              toggleFit();
              closeFab();
            }}
            role="menuitem"
          >
            {fitToWindow ? "Actual Size" : "Fit"}
          </button>
          <button
            class="fab-item"
            onclick={(event) => {
              event.stopPropagation();
              resetZoom();
              closeFab();
            }}
            role="menuitem"
          >
            Reset
          </button>
          <button
            class="fab-item"
            onclick={(event) => {
              event.stopPropagation();
              zoomIn();
              closeFab();
            }}
            role="menuitem"
          >
            Zoom In
          </button>
          <button
            class="fab-item"
            onclick={(event) => {
              event.stopPropagation();
              zoomOut();
              closeFab();
            }}
            role="menuitem"
          >
            Zoom Out
          </button>
        </div>
      {/if}
      <button
        class="fab-main"
        onclick={(event) => {
          event.stopPropagation();
          toggleFab();
        }}
        aria-label="Open controls"
        aria-expanded={isFabOpen}
      >
        +
      </button>
    </div>
  </div>
</main>
